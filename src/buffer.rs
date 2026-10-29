use ropey::Rope;

/// Text buffer backed by a rope. Positions are char indices.
/// `head` is the cursor, `anchor` the other end of the selection.
pub struct Buffer {
    rope: Rope,
    head: usize,
    anchor: usize,
    goal_col: Option<usize>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            head: 0,
            anchor: 0,
            goal_col: None,
        }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
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

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(s, e)| self.rope.slice(s..e).to_string())
    }

    // --- editing ---

    pub fn delete_selection(&mut self) -> bool {
        if let Some((s, e)) = self.selection() {
            self.rope.remove(s..e);
            self.head = s;
            self.anchor = s;
            true
        } else {
            false
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.delete_selection();
        self.rope.insert(self.head, text);
        self.head += text.chars().count();
        self.anchor = self.head;
        self.goal_col = None;
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
            self.rope.remove(start..self.head);
            self.head = start;
            self.anchor = start;
        }
        self.goal_col = None;
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
            self.rope.remove(self.head..end);
        }
        self.goal_col = None;
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
    fn crlf_handling() {
        let mut b = Buffer::new("a\r\nb");
        assert_eq!(b.line_text(0), "a");
        b.set_cursor(1, 0, false);
        b.backspace();
        assert_eq!(b.text(), "ab");
    }
}
