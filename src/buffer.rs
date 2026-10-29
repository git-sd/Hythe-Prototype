use ropey::Rope;

/// Text buffer backed by a rope. Positions are char indices.
/// `head` is the cursor, `anchor` the other end of the selection.
pub struct Buffer {
    rope: Rope,
    head: usize,
    anchor: usize,
    goal_col: Option<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Record edits for collaboration (see `drain_edits`).
    track: bool,
    outbox: Vec<EditOp>,
}

/// One edit as a collaboration peer sees it: UTF-16 offsets, replace `remove` units at `pos` with `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOp {
    pub pos: u32,
    pub remove: u32,
    pub insert: String,
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// One reversible change: `removed` was replaced by `inserted` at `pos`.
/// `before`/`after` are (head, anchor) around the change.
struct Edit {
    pos: usize,
    removed: String,
    inserted: String,
    before: (usize, usize),
    after: (usize, usize),
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            head: 0,
            anchor: 0,
            goal_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            track: false,
            outbox: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    // --- collaboration ---

    /// Start or stop recording edits for `drain_edits`.
    pub fn track_edits(&mut self, on: bool) {
        self.track = on;
        if !on {
            self.outbox.clear();
        }
    }

    /// Edits made since the last call, oldest first.
    pub fn drain_edits(&mut self) -> Vec<EditOp> {
        std::mem::take(&mut self.outbox)
    }

    /// Cursor and selection anchor as UTF-16 offsets into the document.
    pub fn cursor_utf16(&self) -> (u32, u32) {
        (self.rope.char_to_utf16_cu(self.head) as u32, self.rope.char_to_utf16_cu(self.anchor) as u32)
    }

    /// Character index for a UTF-16 offset (clamped to the document).
    pub fn utf16_to_char(&self, offset: u32) -> usize {
        self.rope.utf16_cu_to_char((offset as usize).min(self.rope.len_utf16_cu()))
    }

    /// Apply edits made by someone else: `(utf16 position, utf16 units to remove, text)`,
    /// each in the text as left by the previous one. Our cursor and selection move
    /// with the text. Undo history is dropped, since its positions no longer apply.
    pub fn apply_remote<'a>(&mut self, changes: impl IntoIterator<Item = (u32, u32, &'a str)>) {
        fn shift(pos: usize, start: usize, removed: usize, inserted: usize) -> usize {
            if pos <= start {
                pos
            } else if pos < start + removed {
                start + inserted
            } else {
                pos - removed + inserted
            }
        }
        for (pos, remove, insert) in changes {
            let units = self.rope.len_utf16_cu();
            let start = self.rope.utf16_cu_to_char((pos as usize).min(units));
            let end = self.rope.utf16_cu_to_char(((pos + remove) as usize).min(units)).max(start);
            let removed = end - start;
            let inserted = insert.chars().count();
            if removed > 0 {
                self.rope.remove(start..end);
            }
            if inserted > 0 {
                self.rope.insert(start, insert);
            }
            self.head = shift(self.head, start, removed, inserted);
            self.anchor = shift(self.anchor, start, removed, inserted);
        }
        self.undo.clear();
        self.redo.clear();
        self.goal_col = None;
    }

    pub fn byte_len(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_start_byte(&self, line: usize) -> usize {
        self.rope.char_to_byte(self.rope.line_to_char(line))
    }

    /// Line contents without the trailing line break.
    pub fn line_text(&self, line: usize) -> String {
        let mut s = self.rope.line(line).to_string();
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        s
    }

    fn line_len(&self, line: usize) -> usize {
        self.line_text(line).chars().count()
    }

    pub fn cursor(&self) -> (usize, usize) {
        let line = self.rope.char_to_line(self.head);
        (line, self.head - self.rope.line_to_char(line))
    }

    /// Selection as ordered (start, end) char indices, if non-empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.head != self.anchor).then(|| (self.head.min(self.anchor), self.head.max(self.anchor)))
    }

    /// Selected range clipped to one line, as (start_col, end_col) where
    /// `end_col` may equal line length + 1 to show the selected line break.
    pub fn selection_on_line(&self, line: usize) -> Option<(usize, usize)> {
        let (s, e) = self.selection()?;
        let ls = self.rope.line_to_char(line);
        let len = self.line_len(line);
        let le = ls + len;
        if e <= ls || s > le {
            return None;
        }
        let sc = s.max(ls) - ls;
        let ec = if e > le { len + 1 } else { e - ls };
        (sc < ec).then_some((sc, ec))
    }

    /// Cursor as a character index into the whole document.
    pub fn cursor_index(&self) -> usize {
        self.head
    }

    pub fn has_selection(&self) -> bool {
        self.head != self.anchor
    }

    pub fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte(char_idx.min(self.rope.len_chars()))
    }

    pub fn byte_to_char(&self, byte_idx: usize) -> usize {
        self.rope.byte_to_char(byte_idx.min(self.rope.len_bytes()))
    }

    /// (line, column) of a character index.
    pub fn line_col(&self, char_idx: usize) -> (usize, usize) {
        let idx = char_idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        (line, idx - self.rope.line_to_char(line))
    }

    /// Replace `start..cursor` with `text` as one undoable edit, then select
    /// `select` (character offsets within `text`). Used to accept a completion.
    pub fn apply_completion(&mut self, start: usize, text: &str, select: (usize, usize)) {
        let start = start.min(self.head);
        self.replace(start, self.head, text);
        self.anchor = start + select.0;
        self.head = start + select.1;
        if let Some(last) = self.undo.last_mut() {
            last.after = (self.head, self.anchor);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(s, e)| self.rope.slice(s..e).to_string())
    }

    // --- editing ---

    /// Replace `start..end` with `text`, leaving the caret after it and
    /// recording the change for undo. Runs of typing or deleting coalesce.
    fn replace(&mut self, start: usize, end: usize, text: &str) {
        let removed = self.rope.slice(start..end).to_string();
        if self.track {
            self.outbox.push(EditOp {
                pos: self.rope.char_to_utf16_cu(start) as u32,
                remove: utf16_len(&removed),
                insert: text.to_string(),
            });
        }
        let before = (self.head, self.anchor);
        if end > start {
            self.rope.remove(start..end);
        }
        if !text.is_empty() {
            self.rope.insert(start, text);
        }
        let caret = start + text.chars().count();
        self.head = caret;
        self.anchor = caret;
        self.goal_col = None;
        self.redo.clear();

        let edit = Edit {
            pos: start,
            removed,
            inserted: text.to_string(),
            before,
            after: (caret, caret),
        };
        if !self.try_merge(&edit) {
            self.undo.push(edit);
        }
    }

    fn try_merge(&mut self, e: &Edit) -> bool {
        let Some(last) = self.undo.last_mut() else {
            return false;
        };
        if last.after != e.before || e.before.0 != e.before.1 {
            return false;
        }
        let typing = last.removed.is_empty()
            && e.removed.is_empty()
            && e.inserted.chars().count() == 1
            && e.inserted != "\n"
            && !last.inserted.ends_with('\n');
        let backspacing = last.inserted.is_empty()
            && e.inserted.is_empty()
            && e.pos + e.removed.chars().count() == last.pos;
        let deleting = last.inserted.is_empty() && e.inserted.is_empty() && e.pos == last.pos;
        if typing {
            last.inserted.push_str(&e.inserted);
        } else if backspacing {
            last.pos = e.pos;
            last.removed = format!("{}{}", e.removed, last.removed);
        } else if deleting {
            last.removed.push_str(&e.removed);
        } else {
            return false;
        }
        last.after = e.after;
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else {
            return false;
        };
        let n = e.inserted.chars().count();
        if self.track {
            self.outbox.push(EditOp {
                pos: self.rope.char_to_utf16_cu(e.pos) as u32,
                remove: utf16_len(&e.inserted),
                insert: e.removed.clone(),
            });
        }
        if n > 0 {
            self.rope.remove(e.pos..e.pos + n);
        }
        if !e.removed.is_empty() {
            self.rope.insert(e.pos, &e.removed);
        }
        (self.head, self.anchor) = e.before;
        self.goal_col = None;
        self.redo.push(e);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else {
            return false;
        };
        let n = e.removed.chars().count();
        if self.track {
            self.outbox.push(EditOp {
                pos: self.rope.char_to_utf16_cu(e.pos) as u32,
                remove: utf16_len(&e.removed),
                insert: e.inserted.clone(),
            });
        }
        if n > 0 {
            self.rope.remove(e.pos..e.pos + n);
        }
        if !e.inserted.is_empty() {
            self.rope.insert(e.pos, &e.inserted);
        }
        (self.head, self.anchor) = e.after;
        self.goal_col = None;
        self.undo.push(e);
        true
    }

    pub fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((s, e)) => {
                self.replace(s, e, "");
                true
            }
            None => false,
        }
    }

    pub fn insert(&mut self, text: &str) {
        let (s, e) = self.selection().unwrap_or((self.head, self.head));
        self.replace(s, e, text);
    }

    /// Newline that keeps the current line's indentation.
    pub fn insert_newline(&mut self) {
        let (line, _) = self.cursor();
        let indent: String = self
            .line_text(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.insert(&format!("\n{indent}"));
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.head > 0 {
            let start = if self.rope.char(self.head - 1) == '\n'
                && self.head >= 2
                && self.rope.char(self.head - 2) == '\r'
            {
                self.head - 2
            } else {
                self.head - 1
            };
            self.replace(start, self.head, "");
        }
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.head < self.rope.len_chars() {
            let end = if self.rope.char(self.head) == '\r'
                && self.head + 1 < self.rope.len_chars()
                && self.rope.char(self.head + 1) == '\n'
            {
                self.head + 2
            } else {
                self.head + 1
            };
            self.replace(self.head, end, "");
        }
    }

    // --- movement ---

    fn move_to(&mut self, pos: usize, select: bool, keep_goal: bool) {
        self.head = pos.min(self.rope.len_chars());
        if !select {
            self.anchor = self.head;
        }
        if !keep_goal {
            self.goal_col = None;
        }
    }

    pub fn move_left(&mut self, select: bool) {
        if !select {
            if let Some((s, _)) = self.selection() {
                self.move_to(s, false, false);
                return;
            }
        }
        self.move_to(self.head.saturating_sub(1), select, false);
    }

    pub fn move_right(&mut self, select: bool) {
        if !select {
            if let Some((_, e)) = self.selection() {
                self.move_to(e, false, false);
                return;
            }
        }
        self.move_to(self.head + 1, select, false);
    }

    pub fn move_vertical(&mut self, delta: isize, select: bool) {
        let (line, col) = self.cursor();
        let goal = self.goal_col.unwrap_or(col);
        let target = line as isize + delta;
        if target < 0 {
            self.move_to(0, select, true);
        } else if target as usize >= self.line_count() {
            self.move_to(self.rope.len_chars(), select, true);
        } else {
            let t = target as usize;
            let pos = self.rope.line_to_char(t) + goal.min(self.line_len(t));
            self.move_to(pos, select, true);
        }
        self.goal_col = Some(goal);
    }

    pub fn home(&mut self, select: bool) {
        let (line, col) = self.cursor();
        let text = self.line_text(line);
        let indent = text.chars().take_while(|c| c.is_whitespace()).count();
        let target = if col == indent { 0 } else { indent };
        self.move_to(self.rope.line_to_char(line) + target, select, false);
    }

    pub fn end(&mut self, select: bool) {
        let (line, _) = self.cursor();
        self.move_to(self.rope.line_to_char(line) + self.line_len(line), select, false);
    }

    pub fn doc_start(&mut self, select: bool) {
        self.move_to(0, select, false);
    }

    pub fn doc_end(&mut self, select: bool) {
        self.move_to(self.rope.len_chars(), select, false);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.rope.len_chars();
    }

    pub fn set_cursor(&mut self, line: usize, col: usize, select: bool) {
        let line = line.min(self.line_count() - 1);
        let pos = self.rope.line_to_char(line) + col.min(self.line_len(line));
        self.move_to(pos, select, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace() {
        let mut b = Buffer::new("");
        b.insert("hi");
        b.insert_newline();
        b.insert("yo");
        assert_eq!(b.text(), "hi\nyo");
        b.backspace();
        b.backspace();
        b.backspace();
        assert_eq!(b.text(), "hi");
    }

    #[test]
    fn auto_indent_and_vertical_goal_col() {
        let mut b = Buffer::new("    long line\nx\n    another");
        b.set_cursor(0, 9, false);
        b.move_vertical(1, false);
        assert_eq!(b.cursor(), (1, 1));
        b.move_vertical(1, false);
        assert_eq!(b.cursor(), (2, 9));
        b.set_cursor(0, 13, false);
        b.insert_newline();
        assert_eq!(b.cursor(), (1, 4));
    }

    #[test]
    fn selection_replace_and_lines() {
        let mut b = Buffer::new("abc\ndef");
        b.set_cursor(0, 1, false);
        b.set_cursor(1, 2, true);
        assert_eq!(b.selected_text().as_deref(), Some("bc\nde"));
        assert_eq!(b.selection_on_line(0), Some((1, 4)));
        assert_eq!(b.selection_on_line(1), Some((0, 2)));
        b.insert("X");
        assert_eq!(b.text(), "aXf");
    }

    #[test]
    fn undo_redo_coalesces_typing() {
        let mut b = Buffer::new("");
        for c in ["a", "b", "c"] {
            b.insert(c);
        }
        b.insert_newline();
        b.insert("d");
        assert_eq!(b.text(), "abc\nd");
        assert!(b.undo()); // "d"
        assert_eq!(b.text(), "abc\n");
        assert!(b.undo()); // newline
        assert!(b.undo()); // "abc" in one step
        assert_eq!(b.text(), "");
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.text(), "abc");
        b.insert("!");
        assert!(!b.redo());
    }

    #[test]
    fn undo_restores_deleted_selection() {
        let mut b = Buffer::new("hello world");
        b.set_cursor(0, 0, false);
        b.set_cursor(0, 5, true);
        b.insert("bye");
        assert_eq!(b.text(), "bye world");
        b.undo();
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.selected_text().as_deref(), Some("hello"));
        b.backspace();
        b.backspace();
        b.undo();
        assert_eq!(b.text(), "hello world");
    }

    #[test]
    fn completion_is_one_undo_step_and_selects_placeholder() {
        let mut b = Buffer::new("");
        for c in ["f", "o", "r"] {
            b.insert(c);
        }
        let start = 0;
        b.apply_completion(start, "for x in  {}", (4, 5));
        assert_eq!(b.text(), "for x in  {}");
        assert_eq!(b.selected_text().as_deref(), Some("x"));
        // Typing replaces the selected placeholder.
        b.insert("item");
        assert_eq!(b.text(), "for item in  {}");
        b.undo();
        b.undo();
        assert_eq!(b.text(), "for");
        assert_eq!(b.line_col(2), (0, 2));
    }

    #[test]
    fn tracked_edits_replay_into_another_buffer_with_unicode_and_undo() {
        let mut a = Buffer::new("héllo");
        a.track_edits(true);
        a.doc_end(false);
        a.insert(" 😀 world");
        a.insert_newline();
        a.insert("end");
        a.set_cursor(0, 1, false);
        a.delete();
        a.undo();
        a.undo();
        a.redo();
        let mut b = Buffer::new("héllo");
        let ops = a.drain_edits();
        assert!(!ops.is_empty());
        b.apply_remote(ops.iter().map(|o| (o.pos, o.remove, o.insert.as_str())));
        assert_eq!(b.text(), a.text());
        assert!(a.drain_edits().is_empty());
    }

    #[test]
    fn remote_edits_move_the_cursor_with_the_text() {
        let mut b = Buffer::new("hello world");
        b.set_cursor(0, 6, false);
        // Someone inserts before our cursor, then deletes a range around part of it.
        b.apply_remote([(0, 0, ">>> ")]);
        assert_eq!(b.cursor(), (0, 10));
        // Deleting "hello " (which ends exactly at the cursor) pulls the cursor back to the gap.
        b.apply_remote([(4, 6, "")]);
        assert_eq!(b.text(), ">>> world");
        assert_eq!(b.cursor(), (0, 4));
        // Inserting exactly at the cursor leaves the cursor before the new text.
        let (_, col) = b.cursor();
        b.apply_remote([(col as u32, 0, "XY")]);
        assert_eq!(b.cursor().1, col);
        // Our own undo history is not applied on top of foreign edits.
        assert!(!b.undo());
    }

    #[test]
    fn untracked_buffers_record_nothing() {
        let mut a = Buffer::new("");
        a.insert("x");
        assert!(a.drain_edits().is_empty());
        a.track_edits(true);
        a.insert("y");
        a.track_edits(false);
        assert!(a.drain_edits().is_empty());
    }
    #[test]
    fn crlf_handling() {
        let mut b = Buffer::new("a\r\nb");
        assert_eq!(b.line_text(0), "a");
        b.set_cursor(1, 0, false);
        b.backspace();
        assert_eq!(b.text(), "ab");
    }
}
