//! Searching: the in-file find/replace bar, go to line, and project-wide
//! search. The matching itself is plain functions (tested below); the rest is
//! the state, key handling and views that hang them on the editor.

use std::ops::Range;
use std::path::{Path, PathBuf};

use iced::widget::{button, column, container, mouse_area, opaque, row, space, text};
use iced::{clipboard, keyboard, Alignment, Element, Length, Padding, Task, Theme};
use iced::keyboard::{key::Named, Key};
use regex::{Regex, RegexBuilder};

use crate::icons::Icon;
use crate::view::{divider_h, icon_button, vscroll};
use crate::{theme, App, Message};

/// Most matches kept for one file's find bar.
const MAX_MATCHES: usize = 20_000;
/// Project search limits.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_HITS: usize = 2_000;
const MAX_HITS_PER_FILE: usize = 200;
const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git"];

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    /// Match case exactly.
    pub case: bool,
    /// Only whole words.
    pub word: bool,
    /// The query is a regular expression.
    pub regex: bool,
}

/// A compiled query.
pub struct Matcher {
    re: Regex,
    word: bool,
    literal: bool,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Matcher {
    pub fn new(query: &str, opts: &Opts) -> Result<Self, String> {
        if query.is_empty() {
            return Err("empty query".into());
        }
        let pattern = if opts.regex { query.to_string() } else { regex::escape(query) };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!opts.case)
            .multi_line(true)
            .size_limit(10 << 20)
            .build()
            .map_err(|e| {
                // The first line of the error is the useful one.
                let msg = e.to_string();
                msg.lines().last().unwrap_or("invalid pattern").trim().trim_start_matches("error: ").to_string()
            })?;
        Ok(Self { re, word: opts.word, literal: !opts.regex })
    }

    /// A match is a whole word when neither edge touches a word character
    /// next to it (edges that are themselves punctuation need no boundary).
    fn word_ok(text: &str, r: &Range<usize>) -> bool {
        let first = text[r.start..].chars().next();
        let before = text[..r.start].chars().next_back();
        if first.is_some_and(is_word) && before.is_some_and(is_word) {
            return false;
        }
        let last = text[..r.end].chars().next_back();
        let after = text[r.end..].chars().next();
        !(last.is_some_and(is_word) && after.is_some_and(is_word))
    }

    /// Byte ranges of the matches in `text` (empty matches are skipped).
    pub fn find_bytes(&self, text: &str, cap: usize) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        for m in self.re.find_iter(text) {
            if m.start() == m.end() {
                continue;
            }
            let r = m.range();
            if self.word && !Self::word_ok(text, &r) {
                continue;
            }
            out.push(r);
            if out.len() >= cap {
                break;
            }
        }
        out
    }

    /// Matches as char index ranges, which is what the buffer uses.
    pub fn find_chars(&self, text: &str, cap: usize) -> Vec<(usize, usize)> {
        to_char_ranges(text, &self.find_bytes(text, cap))
    }

    /// What `range` (a match in `text`) becomes when replaced by `repl`. In
    /// regex mode `$1` and `${name}` refer to capture groups.
    pub fn replacement(&self, text: &str, range: &Range<usize>, repl: &str) -> String {
        if self.literal {
            return repl.to_string();
        }
        match self.re.captures_at(text, range.start) {
            Some(c) if c.get(0).is_some_and(|m| m.start() == range.start) => {
                let mut out = String::new();
                c.expand(repl, &mut out);
                out
            }
            _ => repl.to_string(),
        }
    }

    /// Replace every match. Returns the char span that changes and its new
    /// text, so the buffer can apply it as one edit.
    pub fn replace_all(&self, text: &str, repl: &str) -> Option<(usize, usize, String, usize)> {
        let ranges = self.find_bytes(text, usize::MAX);
        let (first, last) = (ranges.first()?, ranges.last()?);
        let mut out = String::new();
        let mut at = first.start;
        for r in &ranges {
            out.push_str(&text[at..r.start]);
            out.push_str(&self.replacement(text, r, repl));
            at = r.end;
        }
        let start = text[..first.start].chars().count();
        let end = start + text[first.start..last.end].chars().count();
        Some((start, end, out, ranges.len()))
    }
}

/// Convert sorted, non-overlapping byte ranges to char index ranges.
pub fn to_char_ranges(text: &str, ranges: &[Range<usize>]) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(ranges.len());
    let (mut byte, mut chars) = (0usize, 0usize);
    for r in ranges {
        chars += text[byte..r.start].chars().count();
        let start = chars;
        chars += text[r.start..r.end].chars().count();
        byte = r.end;
        out.push((start, chars));
    }
    out
}

// ---------------------------------------------------------------------------
// Project search (pure part)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Hit {
    /// Zero-based line and char column of the match.
    pub line: usize,
    pub col: usize,
    /// Match length in chars.
    pub len: usize,
    /// The line, trimmed and shortened, and where the match sits in it (chars).
    pub preview: String,
    pub pcol: usize,
    pub plen: usize,
}

#[derive(Clone, Debug)]
pub struct FileHits {
    pub path: PathBuf,
    pub rel: String,
    pub hits: Vec<Hit>,
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub files: Vec<FileHits>,
    pub truncated: bool,
    pub scanned: usize,
}

const PREVIEW_CHARS: usize = 180;

/// Matches in `text`, line by line.
pub fn scan_text(text: &str, m: &Matcher, cap: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        for r in m.find_bytes(line, cap) {
            let col = line[..r.start].chars().count();
            let len = line[r.start..r.end].chars().count();
            let indent = line.chars().take_while(|c| c.is_whitespace()).count().min(col);
            let preview: String = line.chars().skip(indent).take(PREVIEW_CHARS).collect();
            let pcol = col - indent;
            let plen = len.min(PREVIEW_CHARS.saturating_sub(pcol));
            hits.push(Hit { line: line_no, col, len, preview, pcol: pcol.min(PREVIEW_CHARS), plen });
            if hits.len() >= cap {
                return hits;
            }
        }
    }
    hits
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ty) = e.file_type() else { continue };
        if ty.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                walk(&e.path(), root, out);
            }
        } else if ty.is_file() {
            if let Ok(rel) = e.path().strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

/// Files to search, relative to `root`: what git tracks plus untracked files
/// that are not ignored, or (outside a repository) a plain walk.
pub fn list_files(root: &Path) -> Vec<PathBuf> {
    let git = std::process::Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success());
    let mut files: Vec<PathBuf> = match git {
        Some(o) => o
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
            .collect(),
        None => {
            let mut v = Vec::new();
            walk(root, root, &mut v);
            v
        }
    };
    files.retain(|p| {
        !p.components().any(|c| SKIP_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
    });
    files.sort();
    files.dedup();
    files
}

pub fn search_project(root: &Path, query: &str, opts: Opts) -> Result<Outcome, String> {
    let m = Matcher::new(query, &opts)?;
    let mut out = Outcome::default();
    let mut total = 0;
    for rel in list_files(root) {
        let path = root.join(&rel);
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        // Binary files have NULs near the start; other encodings aren't searched.
        if bytes[..bytes.len().min(8000)].contains(&0) {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else { continue };
        out.scanned += 1;
        let room = MAX_HITS - total;
        let hits = scan_text(&text, &m, room.min(MAX_HITS_PER_FILE));
        if hits.is_empty() {
            continue;
        }
        total += hits.len();
        out.files.push(FileHits { rel: rel.to_string_lossy().replace('\\', "/"), path, hits });
        if total >= MAX_HITS {
            out.truncated = true;
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// State and messages
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Query,
    Replace,
    /// Keyboard focus is back in the text.
    Editor,
}

#[derive(Clone, Debug)]
pub enum FindMsg {
    Open { replace: bool },
    Close,
    Next,
    Prev,
    Case,
    Word,
    Regex,
    ToggleReplace,
    Focus(Field),
    Replace,
    ReplaceAll,
    Paste(Option<String>),
}

pub struct Find {
    pub open: bool,
    pub query: String,
    pub replace: String,
    pub opts: Opts,
    pub show_replace: bool,
    pub focus: Field,
    /// Matches in the focused document, as char ranges.
    pub matches: Vec<(usize, usize)>,
    pub current: Option<usize>,
    pub error: Option<String>,
    key: Option<(usize, u64, String, Opts)>,
}

impl Default for Find {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            replace: String::new(),
            opts: Opts::default(),
            show_replace: false,
            focus: Field::Query,
            matches: Vec::new(),
            current: None,
            error: None,
            key: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ProjMsg {
    Open,
    Close,
    Case,
    Word,
    Regex,
    Run,
    Done(u64, Result<Outcome, String>),
    Pick(usize, usize),
    Paste(Option<String>),
}

#[derive(Default)]
pub struct ProjSearch {
    pub open: bool,
    pub query: String,
    pub opts: Opts,
    pub results: Outcome,
    pub running: bool,
    pub searched: bool,
    pub error: Option<String>,
    gen: u64,
}

// ---------------------------------------------------------------------------
// App integration
// ---------------------------------------------------------------------------

impl App {
    /// Scroll the focused pane so `line` is on screen, centred if it was not.
    fn reveal_line_centered(&mut self, line: usize) {
        let Some(pane) = self.panes.list.get_mut(self.panes.focus) else { return };
        let vp = pane.viewport.get();
        let Some(tab) = pane.tabs.get_mut(pane.active) else { return };
        let Some(d) = self.docs.get(tab.doc) else { return };
        let h = tab.scroll.metrics().line_h;
        let y = line as f32 * h;
        if y < tab.scroll.y || y + h > tab.scroll.y + vp.height {
            tab.scroll.y = (y - vp.height / 2.0).max(0.0);
        }
        tab.scroll.ensure_cursor_visible(&d.buffer, vp);
    }

    /// Put the caret at 1-based `line`/`col` (col 0: line start).
    pub(crate) fn go_to_line(&mut self, line: usize, col: usize) {
        self.focus_term = false;
        let Some(d) = self.docs.get_mut(self.panes.cur()) else { return };
        let line = line.clamp(1, d.buffer.line_count()) - 1;
        d.buffer.set_cursor(line, col.saturating_sub(1), false);
        self.reveal_line_centered(line);
        self.status = format!("Line {}", line + 1);
    }

    /// Select `len` chars at 0-based `line`/`col` and scroll there.
    fn select_at(&mut self, line: usize, col: usize, len: usize) {
        self.focus_term = false;
        let Some(d) = self.docs.get_mut(self.panes.cur()) else { return };
        let line = line.min(d.buffer.line_count() - 1);
        let start = d.buffer.line_start_char(line) + col;
        d.buffer.select_range(start, start + len);
        self.reveal_line_centered(line);
    }

    /// Recompute the matches if the query, options, document or text changed
    /// (`force` also jumps to the match at or after the caret).
    pub(crate) fn find_recompute(&mut self, force: bool) {
        if !self.find.open {
            return;
        }
        let di = self.panes.cur();
        let f = &mut self.find;
        let Some(d) = self.docs.get(di) else {
            f.matches.clear();
            f.current = None;
            f.key = None;
            return;
        };
        let key = (di, d.buffer.revision(), f.query.clone(), f.opts);
        if !force && f.key.as_ref() == Some(&key) {
            return;
        }
        f.key = Some(key);
        f.error = None;
        f.matches.clear();
        f.current = None;
        if f.query.is_empty() {
            return;
        }
        match Matcher::new(&f.query, &f.opts) {
            Ok(m) => f.matches = m.find_chars(&d.buffer.text(), MAX_MATCHES),
            Err(e) => {
                f.error = Some(e);
                return;
            }
        }
        let at = d.buffer.selection().map(|s| s.0).unwrap_or_else(|| d.buffer.cursor_index());
        f.current = f.matches.iter().position(|m| m.0 >= at).or(if f.matches.is_empty() { None } else { Some(0) });
        if force {
            self.find_show_current();
        }
    }

    /// Select the current match and scroll to it.
    fn find_show_current(&mut self) {
        let Some((s, e)) = self.find.current.and_then(|i| self.find.matches.get(i).copied()) else { return };
        let Some(d) = self.docs.get_mut(self.panes.cur()) else { return };
        d.buffer.select_range(s, e);
        let line = d.buffer.line_col(s).0;
        self.reveal_line_centered(line);
    }

    fn find_step(&mut self, forward: bool) {
        self.find_recompute(false);
        let n = self.find.matches.len();
        if n == 0 {
            return;
        }
        let at = self.docs.get(self.panes.cur()).map(|d| d.buffer.selection().map(|s| s.0).unwrap_or(d.buffer.cursor_index()));
        let cur = self.find.current;
        // Move on from the selected match; if the caret has wandered off, start from it.
        let selected = cur.filter(|i| Some(self.find.matches[*i].0) == at);
        let next = match (selected, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => at.and_then(|a| self.find.matches.iter().position(|m| m.0 >= a)).unwrap_or(0),
            (None, false) => at
                .and_then(|a| self.find.matches.iter().rposition(|m| m.0 < a))
                .unwrap_or(n - 1),
        };
        self.find.current = Some(next);
        self.find_show_current();
    }

    fn find_open(&mut self, replace: bool) {
        self.focus_term = false;
        // A one-line selection becomes the query.
        if let Some(t) = self.docs.get(self.panes.cur()).and_then(|d| d.buffer.selected_text()) {
            if !t.contains('\n') && !t.is_empty() && t.chars().count() < 200 {
                self.find.query = t;
            }
        }
        self.find.open = true;
        self.find.focus = if replace && !self.find.query.is_empty() { Field::Replace } else { Field::Query };
        self.find.show_replace |= replace;
        self.find_recompute(true);
    }

    pub(crate) fn find_update(&mut self, msg: FindMsg) -> Task<Message> {
        match msg {
            FindMsg::Open { replace } => self.find_open(replace),
            FindMsg::Close => {
                self.find.open = false;
                self.find.focus = Field::Editor;
                self.find.matches.clear();
                self.find.current = None;
                self.find.key = None;
            }
            FindMsg::Next => self.find_step(true),
            FindMsg::Prev => self.find_step(false),
            FindMsg::Case => {
                self.find.opts.case = !self.find.opts.case;
                self.find_recompute(true);
            }
            FindMsg::Word => {
                self.find.opts.word = !self.find.opts.word;
                self.find_recompute(true);
            }
            FindMsg::Regex => {
                self.find.opts.regex = !self.find.opts.regex;
                self.find_recompute(true);
            }
            FindMsg::ToggleReplace => {
                self.find.show_replace = !self.find.show_replace;
                if !self.find.show_replace && self.find.focus == Field::Replace {
                    self.find.focus = Field::Query;
                }
            }
            FindMsg::Focus(f) => {
                self.focus_term = false;
                self.find.focus = f;
            }
            FindMsg::Replace => self.find_replace_one(),
            FindMsg::ReplaceAll => self.find_replace_all(),
            FindMsg::Paste(Some(t)) => {
                let t = t.replace("\r\n", "\n");
                match self.find.focus {
                    Field::Query => {
                        self.find.query.push_str(t.lines().next().unwrap_or(""));
                        self.find_recompute(true);
                    }
                    Field::Replace => self.find.replace.push_str(&t),
                    Field::Editor => {}
                }
            }
            FindMsg::Paste(None) => {}
        }
        Task::none()
    }

    fn find_replace_one(&mut self) {
        self.find_recompute(false);
        let Some((s, e)) = self.find.current.and_then(|i| self.find.matches.get(i).copied()) else { return };
        let Ok(m) = Matcher::new(&self.find.query, &self.find.opts) else { return };
        let Some(d) = self.docs.get_mut(self.panes.cur()) else { return };
        let text = d.buffer.text();
        let range = d.buffer.char_to_byte(s)..d.buffer.char_to_byte(e);
        let new = m.replacement(&text, &range, &self.find.replace);
        d.buffer.replace_range(s, e, &new);
        d.edited(&mut self.highlighter);
        // Carry on to the next match after the replaced text.
        self.find_recompute(true);
    }

    fn find_replace_all(&mut self) {
        let Ok(m) = Matcher::new(&self.find.query, &self.find.opts) else { return };
        let Some(d) = self.docs.get_mut(self.panes.cur()) else { return };
        let text = d.buffer.text();
        let Some((s, e, new, n)) = m.replace_all(&text, &self.find.replace) else {
            self.status = "No matches".into();
            return;
        };
        d.buffer.replace_range(s, e, &new);
        d.edited(&mut self.highlighter);
        self.status = format!("Replaced {n} match{}", if n == 1 { "" } else { "es" });
        self.find_recompute(true);
    }

    /// Keys while the find bar has focus.
    pub(crate) fn find_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<&str>) -> Task<Message> {
        let shift = mods.shift();
        let field = self.find.focus;
        match key.as_ref() {
            Key::Named(Named::Escape) => return self.find_update(FindMsg::Close),
            Key::Named(Named::Enter) => {
                return self.find_update(match (field, shift) {
                    (Field::Replace, _) => FindMsg::Replace,
                    (_, true) => FindMsg::Prev,
                    _ => FindMsg::Next,
                })
            }
            Key::Named(Named::F3) => return self.find_update(if shift { FindMsg::Prev } else { FindMsg::Next }),
            Key::Named(Named::Tab) => {
                if self.find.show_replace {
                    self.find.focus = if field == Field::Query { Field::Replace } else { Field::Query };
                }
                return Task::none();
            }
            Key::Named(Named::Backspace) => {
                match field {
                    Field::Query => {
                        if mods.command() {
                            trim_word(&mut self.find.query);
                        } else {
                            self.find.query.pop();
                        }
                        self.find_recompute(true);
                    }
                    Field::Replace => {
                        if mods.command() {
                            trim_word(&mut self.find.replace);
                        } else {
                            self.find.replace.pop();
                        }
                    }
                    Field::Editor => {}
                }
                return Task::none();
            }
            Key::Named(Named::ArrowDown) => return self.find_update(FindMsg::Next),
            Key::Named(Named::ArrowUp) => return self.find_update(FindMsg::Prev),
            _ => {}
        }
        if let Key::Character(c) = key.as_ref() {
            let c = c.to_lowercase();
            if mods.alt() && !mods.command() {
                match c.as_str() {
                    "c" => return self.find_update(FindMsg::Case),
                    "w" => return self.find_update(FindMsg::Word),
                    "r" => return self.find_update(FindMsg::Regex),
                    _ => {}
                }
            }
            if mods.command() {
                match (c.as_str(), shift) {
                    ("f", false) => {
                        self.find.focus = Field::Query;
                        return Task::none();
                    }
                    ("h", _) => return self.find_update(FindMsg::Open { replace: true }),
                    ("v", _) => return clipboard::read().map(|t| Message::Find(FindMsg::Paste(t))),
                    ("a", _) | ("c", _) | ("x", _) => return Task::none(),
                    // Everything else (save, palette, …) behaves as in the editor.
                    _ => return self.handle_key(key, mods, text),
                }
            }
        }
        if !mods.command() && !mods.alt() || mods.control() && mods.alt() {
            let t: String = text.unwrap_or("").chars().filter(|c| !c.is_control()).collect();
            if !t.is_empty() {
                match field {
                    Field::Query => {
                        self.find.query.push_str(&t);
                        self.find_recompute(true);
                    }
                    Field::Replace => self.find.replace.push_str(&t),
                    Field::Editor => {}
                }
            }
        }
        Task::none()
    }

    // --- project search ---

    fn proj_run(&mut self) -> Task<Message> {
        let Some(root) = self.root.as_ref().map(|r| r.path.clone()) else {
            self.status = "Open a folder to search it".into();
            return Task::none();
        };
        let ps = &mut self.psearch;
        ps.error = None;
        if ps.query.is_empty() {
            ps.results = Outcome::default();
            ps.searched = false;
            ps.running = false;
            ps.gen += 1;
            return Task::none();
        }
        ps.gen += 1;
        ps.running = true;
        let (gen, query, opts) = (ps.gen, ps.query.clone(), ps.opts);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || search_project(&root, &query, opts))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |r| Message::Project(ProjMsg::Done(gen, r)),
        )
    }

    pub(crate) fn proj_update(&mut self, msg: ProjMsg) -> Task<Message> {
        match msg {
            ProjMsg::Open => {
                self.psearch.open = true;
                if let Some(t) = self.docs.get(self.panes.cur()).and_then(|d| d.buffer.selected_text()) {
                    if !t.contains('\n') && !t.is_empty() && t.chars().count() < 200 {
                        self.psearch.query = t;
                        return self.proj_run();
                    }
                }
            }
            ProjMsg::Close => self.psearch.open = false,
            ProjMsg::Case => {
                self.psearch.opts.case = !self.psearch.opts.case;
                return self.proj_run();
            }
            ProjMsg::Word => {
                self.psearch.opts.word = !self.psearch.opts.word;
                return self.proj_run();
            }
            ProjMsg::Regex => {
                self.psearch.opts.regex = !self.psearch.opts.regex;
                return self.proj_run();
            }
            ProjMsg::Run => return self.proj_run(),
            ProjMsg::Done(gen, result) => {
                if gen != self.psearch.gen {
                    return Task::none();
                }
                self.psearch.running = false;
                self.psearch.searched = true;
                match result {
                    Ok(o) => self.psearch.results = o,
                    Err(e) => {
                        self.psearch.results = Outcome::default();
                        self.psearch.error = Some(e);
                    }
                }
            }
            ProjMsg::Pick(fi, hi) => {
                let hit = self.psearch.results.files.get(fi).and_then(|f| f.hits.get(hi).map(|h| (f.path.clone(), h.clone())));
                if let Some((path, h)) = hit {
                    self.psearch.open = false;
                    self.open_file(path);
                    self.select_at(h.line, h.col, h.len);
                }
            }
            ProjMsg::Paste(Some(t)) => {
                self.psearch.query.push_str(t.lines().next().unwrap_or(""));
                return self.proj_run();
            }
            ProjMsg::Paste(None) => {}
        }
        Task::none()
    }

    pub(crate) fn proj_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<&str>) -> Task<Message> {
        match key.as_ref() {
            Key::Named(Named::Escape) => return self.proj_update(ProjMsg::Close),
            Key::Named(Named::Enter) => return self.proj_update(ProjMsg::Run),
            Key::Named(Named::Backspace) => {
                if mods.command() {
                    trim_word(&mut self.psearch.query);
                } else {
                    self.psearch.query.pop();
                }
                return Task::none();
            }
            _ => {}
        }
        if let Key::Character(c) = key.as_ref() {
            let c = c.to_lowercase();
            if mods.alt() && !mods.command() {
                match c.as_str() {
                    "c" => return self.proj_update(ProjMsg::Case),
                    "w" => return self.proj_update(ProjMsg::Word),
                    "r" => return self.proj_update(ProjMsg::Regex),
                    _ => {}
                }
            }
            if mods.command() {
                return match (c.as_str(), mods.shift()) {
                    ("f", true) => self.proj_update(ProjMsg::Close),
                    ("v", _) => clipboard::read().map(|t| Message::Project(ProjMsg::Paste(t))),
                    _ => Task::none(),
                };
            }
        }
        if !mods.command() && !mods.alt() || mods.control() && mods.alt() {
            let t: String = text.unwrap_or("").chars().filter(|c| !c.is_control()).collect();
            self.psearch.query.push_str(&t);
        }
        Task::none()
    }

    // --- views ---

    /// The find bar shown above the focused pane's text.
    pub(crate) fn find_bar(&self, width: f32) -> Element<'_, Message> {
        let f = &self.find;
        let narrow = width < 470.0;

        let count: Element<'_, Message> = {
            let (label, color) = if let Some(e) = &f.error {
                (format!("Invalid: {e}").chars().take(28).collect::<String>(), theme::REMOVED)
            } else if f.query.is_empty() {
                (String::new(), theme::TEXT_DIM)
            } else if f.matches.is_empty() {
                ("No results".to_string(), theme::REMOVED)
            } else {
                let more = if f.matches.len() >= MAX_MATCHES { "+" } else { "" };
                (format!("{} of {}{more}", f.current.map(|i| i + 1).unwrap_or(0), f.matches.len()), theme::TEXT_DIM)
            };
            container(text(label).size(12).color(color).wrapping(text::Wrapping::None)).padding([0, 4]).into()
        };
        let toggles = row![
            toggle("Aa", "Match case (Alt+C)", f.opts.case, FindMsg::Case),
            toggle("ab", "Whole word (Alt+W)", f.opts.word, FindMsg::Word),
            toggle(".*", "Regex (Alt+R)", f.opts.regex, FindMsg::Regex),
        ]
        .spacing(2);
        let nav = row![
            icon_button(Icon::ArrowUp, theme::TEXT_DIM, false, Message::Find(FindMsg::Prev)),
            icon_button(Icon::ArrowDown, theme::TEXT_DIM, false, Message::Find(FindMsg::Next)),
        ];
        let chevron = icon_button(
            if f.show_replace { Icon::ChevronDown } else { Icon::ChevronRight },
            theme::TEXT_DIM,
            f.show_replace,
            Message::Find(FindMsg::ToggleReplace),
        );
        let close = icon_button(Icon::Close, theme::TEXT_DIM, false, Message::Find(FindMsg::Close));
        let query = field(&f.query, "Find", f.focus == Field::Query, f.error.is_some(), Field::Query);

        let tiny = width < 330.0;
        let mut bar = column![].spacing(4);
        if tiny {
            bar = bar
                .push(row![chevron, query, close].spacing(4).align_y(Alignment::Center))
                .push(row![toggles, nav].spacing(4).align_y(Alignment::Center).padding(Padding::ZERO.left(30.0)))
                .push(container(count).padding(Padding::ZERO.left(30.0)));
        } else if narrow {
            bar = bar
                .push(row![chevron, query, nav, close].spacing(4).align_y(Alignment::Center))
                .push(row![toggles, count].spacing(6).align_y(Alignment::Center).padding(Padding::ZERO.left(34.0)));
        } else {
            bar = bar.push(
                row![chevron, query, toggles, count, nav, close].spacing(4).align_y(Alignment::Center),
            );
        }
        if f.show_replace {
            let replace = field(&f.replace, "Replace", f.focus == Field::Replace, false, Field::Replace);
            let pad = space::horizontal().width(Length::Fixed(30.0));
            if tiny {
                bar = bar
                    .push(row![pad, replace].align_y(Alignment::Center))
                    .push(
                        row![
                            space::horizontal().width(Length::Fixed(30.0)),
                            action("Replace", FindMsg::Replace),
                            action("All", FindMsg::ReplaceAll)
                        ]
                        .spacing(4),
                    );
            } else {
                bar = bar.push(
                    row![pad, replace, action("Replace", FindMsg::Replace), action("Replace All", FindMsg::ReplaceAll)]
                        .spacing(4)
                        .align_y(Alignment::Center),
                );
            }
        }
        column![
            container(bar).padding([5, 8]).width(Length::Fill).style(theme::bg(theme::PANEL_BG)),
            divider_h()
        ]
        .into()
    }

    /// The project search panel, floating over the window like the palette.
    pub(crate) fn psearch_view(&self) -> Element<'_, Message> {
        let ps = &self.psearch;
        let toggles = row![
            ptoggle("Aa", ps.opts.case, ProjMsg::Case),
            ptoggle("ab", ps.opts.word, ProjMsg::Word),
            ptoggle(".*", ps.opts.regex, ProjMsg::Regex),
        ]
        .spacing(2);
        let q = if ps.query.is_empty() { String::new() } else { format!("{}▏", ps.query) };
        let input = container(
            row![
                container(
                    text(if ps.query.is_empty() { "Search in project…".to_string() } else { q })
                        .size(15)
                        .color(if ps.query.is_empty() { theme::TEXT_DIM } else { theme::TEXT })
                        .wrapping(text::Wrapping::None),
                )
                .width(Length::Fill)
                .clip(true),
                toggles
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([8, 14])
        .width(Length::Fill);

        let status = if let Some(e) = &ps.error {
            format!("Invalid pattern: {e}")
        } else if ps.running {
            "Searching…".to_string()
        } else if !ps.searched {
            if self.root.is_some() { "Type a query and press Enter".to_string() } else { "Open a folder to search it".into() }
        } else {
            let n: usize = ps.results.files.iter().map(|f| f.hits.len()).sum();
            if n == 0 {
                format!("No results ({} files searched)", ps.results.scanned)
            } else {
                format!(
                    "{n}{} result{} in {} file{}",
                    if ps.results.truncated { "+" } else { "" },
                    if n == 1 { "" } else { "s" },
                    ps.results.files.len(),
                    if ps.results.files.len() == 1 { "" } else { "s" },
                )
            }
        };
        let status_color = if ps.error.is_some() { theme::REMOVED } else { theme::TEXT_DIM };

        let mut list = column![].padding(4);
        for (fi, file) in ps.results.files.iter().enumerate() {
            list = list.push(
                container(
                    row![
                        text(file.rel.clone()).size(12).color(theme::TEXT).wrapping(text::Wrapping::None),
                        text(file.hits.len().to_string()).size(11).color(theme::TEXT_FAINT),
                    ]
                    .spacing(8),
                )
                .padding(Padding { top: 8.0, right: 8.0, bottom: 2.0, left: 8.0 }),
            );
            for (hi, h) in file.hits.iter().enumerate() {
                let chars: Vec<char> = h.preview.chars().collect();
                let (a, b) = (h.pcol.min(chars.len()), (h.pcol + h.plen).min(chars.len()));
                let part = |r: Range<usize>| chars[r].iter().collect::<String>();
                let mono = |s: String, c| text(s).size(12).font(theme::MONO).color(c).wrapping(text::Wrapping::None);
                list = list.push(
                    button(
                        row![
                            container(text((h.line + 1).to_string()).size(11).font(theme::MONO).color(theme::TEXT_FAINT))
                                .width(Length::Fixed(40.0)),
                            container(
                                row![
                                    mono(part(0..a), theme::TEXT_DIM),
                                    mono(part(a..b), theme::ACCENT),
                                    mono(part(b..chars.len()), theme::TEXT_DIM),
                                ]
                            )
                            .clip(true)
                            .width(Length::Fill),
                        ]
                        .spacing(6),
                    )
                    .width(Length::Fill)
                    .padding([3, 8])
                    .style(theme::flat(false))
                    .on_press(Message::Project(ProjMsg::Pick(fi, hi))),
                );
            }
        }
        let w = (self.win.width - 40.0).clamp(280.0, 760.0);
        let panel = container(column![
            input,
            divider_h(),
            container(text(status).size(12).color(status_color)).padding([5, 14]),
            divider_h(),
            container(vscroll(list)).height(Length::Fill),
        ])
        .width(Length::Fixed(w))
        .height(Length::Fixed((self.win.height - 120.0).clamp(160.0, 600.0)))
        .style(|_: &Theme| container::Style {
            background: Some(theme::PANEL_BG.into()),
            border: iced::Border { color: theme::BORDER, width: 1.0, radius: 4.0.into() },
            ..Default::default()
        });
        container(opaque(panel))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .padding(Padding::ZERO.top(60.0))
            .into()
    }
}

fn trim_word(s: &mut String) {
    while s.ends_with(|c: char| c.is_whitespace()) {
        s.pop();
    }
    while s.ends_with(|c: char| !c.is_whitespace()) {
        s.pop();
    }
}

fn field<'a>(value: &str, placeholder: &'a str, focused: bool, bad: bool, which: Field) -> Element<'a, Message> {
    let empty = value.is_empty();
    let shown = match (empty, focused) {
        (true, true) => "▏".to_string(),
        (true, false) => placeholder.to_string(),
        (false, true) => format!("{value}▏"),
        (false, false) => value.to_string(),
    };
    let color = if empty && !focused { theme::TEXT_FAINT } else { theme::TEXT };
    let border = if bad {
        theme::REMOVED
    } else if focused {
        theme::ACCENT
    } else {
        theme::SELECTED
    };
    mouse_area(
        container(text(shown).size(13).color(color).wrapping(text::Wrapping::None))
            .padding([4, 8])
            .width(Length::Fill)
            .clip(true)
            .style(move |_: &Theme| container::Style {
                background: Some(theme::EDITOR_BG.into()),
                border: iced::Border { color: border, width: 1.0, radius: 3.0.into() },
                ..Default::default()
            }),
    )
    .on_press(Message::Find(FindMsg::Focus(which)))
    .interaction(iced::mouse::Interaction::Text)
    .into()
}

fn toggle<'a>(label: &'static str, _tip: &'static str, on: bool, msg: FindMsg) -> Element<'a, Message> {
    button(text(label).size(12).font(theme::MONO))
        .padding([4, 6])
        .style(theme::flat(on))
        .on_press(Message::Find(msg))
        .into()
}

fn ptoggle<'a>(label: &'static str, on: bool, msg: ProjMsg) -> Element<'a, Message> {
    button(text(label).size(12).font(theme::MONO))
        .padding([4, 6])
        .style(theme::flat(on))
        .on_press(Message::Project(msg))
        .into()
}

fn action<'a>(label: &'static str, msg: FindMsg) -> Element<'a, Message> {
    button(text(label).size(12).wrapping(text::Wrapping::None))
        .padding([4, 8])
        .style(theme::flat(false))
        .on_press(Message::Find(msg))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn find(text: &str, q: &str, case: bool, word: bool, regex: bool) -> Vec<(usize, usize)> {
        Matcher::new(q, &Opts { case, word, regex }).unwrap().find_chars(text, 1000)
    }

    #[test]
    fn case_insensitive_by_default_and_exact_on_request() {
        let t = "Foo foo FOO";
        assert_eq!(find(t, "foo", false, false, false), vec![(0, 3), (4, 7), (8, 11)]);
        assert_eq!(find(t, "foo", true, false, false), vec![(4, 7)]);
    }

    #[test]
    fn whole_word_respects_identifier_boundaries() {
        let t = "cat concat cat_x cat. (cat)";
        assert_eq!(find(t, "cat", true, true, false), vec![(0, 3), (17, 20), (23, 26)]);
        // Punctuation edges need no boundary.
        assert_eq!(find("a.b a.bc", "a.b", true, true, false), vec![(0, 3)]);
    }

    #[test]
    fn plain_queries_are_literal() {
        assert_eq!(find("a.c abc a*c", "a.c", false, false, false), vec![(0, 3)]);
        assert_eq!(find("1+1=2", "1+1", false, false, false), vec![(0, 3)]);
    }

    #[test]
    fn regex_mode_and_errors() {
        assert_eq!(find("a1 b22 c333", r"\d+", false, false, true), vec![(1, 2), (4, 6), (8, 11)]);
        assert_eq!(find("one\ntwo\n", "^t", false, false, true), vec![(4, 5)]);
        // Whole-word also applies to regex matches: "ab" is followed by "1".
        assert!(find("ab12", r"[a-z]+", false, true, true).is_empty());
        assert!(Matcher::new("(", &Opts { regex: true, ..Opts::default() }).is_err());
        assert!(Matcher::new("", &Opts::default()).is_err());
        // Empty matches are not matches.
        assert!(find("abc", "x*", false, false, true).is_empty());
    }

    #[test]
    fn multibyte_text_gives_char_offsets_not_bytes() {
        let t = "héllo wörld 日本語 😀 héllo";
        let m = find(t, "héllo", true, false, false);
        assert_eq!(m, vec![(0, 5), (18, 23)]);
        // And they index the rope correctly.
        let b = Buffer::new(t);
        let r = find(t, "日本語", true, false, false)[0];
        assert_eq!(b.line_col(r.0), (0, 12));
        assert_eq!(find("日本語日本", "日本", true, true, false), Vec::<(usize, usize)>::new());
    }

    #[test]
    fn replacement_edits_report_utf16_offsets() {
        // The emoji is 2 UTF-16 units but 1 char; the collaboration op must count units.
        let mut b = Buffer::new("😀 foo 😀 foo");
        b.track_edits(true);
        let m = Matcher::new("foo", &Opts::default()).unwrap();
        let (s, e, new, n) = m.replace_all(&b.text(), "x").unwrap();
        assert_eq!((s, e, n), (2, 11, 2));
        b.replace_range(s, e, &new);
        assert_eq!(b.text(), "😀 x 😀 x");
        let ops = b.drain_edits();
        assert_eq!(ops.len(), 1);
        assert_eq!((ops[0].pos, ops[0].remove, ops[0].insert.as_str()), (3, 10, "x 😀 x"));
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let mut b = Buffer::new("a b a b a");
        b.insert("zz"); // some earlier typing
        let m = Matcher::new("a", &Opts { case: true, ..Opts::default() }).unwrap();
        let before = b.text();
        let (s, e, new, _) = m.replace_all(&before, "Q").unwrap();
        b.replace_range(s, e, &new);
        assert_eq!(b.text(), before.replace('a', "Q"));
        assert!(b.undo());
        assert_eq!(b.text(), before);
        assert!(b.redo());
        assert_eq!(b.text(), before.replace('a', "Q"));
    }

    #[test]
    fn sealed_replacement_does_not_merge_with_following_typing() {
        let mut b = Buffer::new("x");
        b.replace_range(0, 1, "y");
        b.insert("a");
        b.insert("b");
        assert!(b.undo()); // undoes the typing of "ab" only
        assert_eq!(b.text(), "y");
        assert!(b.undo());
        assert_eq!(b.text(), "x");
    }

    #[test]
    fn regex_replace_expands_captures_literal_does_not() {
        let re = Matcher::new(r"(\w+)@(\w+)", &Opts { regex: true, ..Opts::default() }).unwrap();
        let (_, _, out, _) = re.replace_all("bob@home x", "$2:$1").unwrap();
        assert_eq!(out, "home:bob");
        let lit = Matcher::new("a", &Opts::default()).unwrap();
        let (_, _, out, _) = lit.replace_all("a", "$1").unwrap();
        assert_eq!(out, "$1");
    }

    #[test]
    fn scan_text_reports_lines_columns_and_previews() {
        let m = Matcher::new("wörld", &Opts::default()).unwrap();
        let hits = scan_text("x\n    héllo wörld\r\nnone\n", &m, 10);
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!((h.line, h.col, h.len), (1, 10, 5));
        assert_eq!(h.preview, "héllo wörld");
        assert_eq!((h.pcol, h.plen), (6, 5));
    }

    #[test]
    fn project_search_skips_ignored_binary_and_big_files_and_caps() {
        let dir = std::env::temp_dir().join(format!("hythe-find-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn needle() {}\n// Needle again\n").unwrap();
        std::fs::write(dir.join("target/b.rs"), "needle").unwrap();
        std::fs::write(dir.join("node_modules/x/c.js"), "needle").unwrap();
        std::fs::write(dir.join("bin.dat"), b"needle\0\0\0").unwrap();
        std::fs::write(dir.join("big.txt"), "needle ".repeat(400_000)).unwrap();
        std::fs::write(dir.join("many.txt"), "needle\n".repeat(5000)).unwrap();

        let out = search_project(&dir, "needle", Opts::default()).unwrap();
        let names: Vec<_> = out.files.iter().map(|f| f.rel.as_str()).collect();
        assert!(names.contains(&"src/a.rs"));
        assert!(!names.iter().any(|n| n.starts_with("target") || n.starts_with("node_modules")));
        assert!(!names.contains(&"bin.dat") && !names.contains(&"big.txt"));
        let a = out.files.iter().find(|f| f.rel == "src/a.rs").unwrap();
        assert_eq!(a.hits.len(), 2);
        let many = out.files.iter().find(|f| f.rel == "many.txt").unwrap();
        assert_eq!(many.hits.len(), MAX_HITS_PER_FILE);

        let cs = search_project(&dir, "needle", Opts { case: true, ..Opts::default() }).unwrap();
        assert_eq!(cs.files.iter().find(|f| f.rel == "src/a.rs").unwrap().hits.len(), 1);
        assert!(search_project(&dir, "(", Opts { regex: true, ..Opts::default() }).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
