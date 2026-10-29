use std::path::{Path, PathBuf};
use std::time::SystemTime;

use iced::Size;

use crate::buffer::Buffer;
use crate::highlight::{Highlighter, Syntax, MAX_HIGHLIGHT_BYTES};

pub const FONT_SIZE: f32 = 14.0;
pub const LINE_H: f32 = 22.0;
pub const CHAR_W: f32 = FONT_SIZE * 0.6;
pub const GUTTER_COLS: usize = 5;
pub const TEXT_X: f32 = GUTTER_COLS as f32 * CHAR_W + 12.0;
pub const TAB_WIDTH: usize = 4;

/// One open file: its text and highlighting. Shared by every pane showing it.
pub struct Doc {
    pub buffer: Buffer,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub classes: Vec<u8>,
    /// Last-change info per line, when inline blame is on.
    pub blame: Option<Vec<hythe_git::BlameLine>>,
    /// Modified time and size of the file as last read or written by us, to
    /// notice when something else (an agent, say) changes it.
    pub stamp: Option<(SystemTime, u64)>,
}

/// Modified time and size of a file, if it exists.
pub fn disk_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl Doc {
    pub fn empty() -> Self {
        Self::from_text("", None)
    }

    pub fn open(path: PathBuf, hl: &mut Highlighter) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(&path)?;
        let stamp = disk_stamp(&path);
        let mut doc = Self::from_text(&text, Some(path));
        doc.stamp = stamp;
        doc.rehighlight(hl);
        Ok(doc)
    }

    fn from_text(text: &str, path: Option<PathBuf>) -> Self {
        Self {
            buffer: Buffer::new(text),
            path,
            dirty: false,
            classes: Vec::new(),
            blame: None,
            stamp: None,
        }
    }

    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into())
    }

    /// Language for completion, from the file extension.
    pub fn lang(&self) -> hythe_complete::Language {
        self.path
            .as_deref()
            .map(hythe_complete::Language::from_path)
            .unwrap_or(hythe_complete::Language::Plain)
    }

    /// Name shown in the status bar.
    pub fn language_name(&self) -> &'static str {
        match self.path {
            Some(_) => self.lang().name(),
            None => "Plain Text",
        }
    }

    /// Grammar used to colour this file. An unsaved new file is treated as Rust.
    pub fn syntax(&self) -> Option<Syntax> {
        match &self.path {
            Some(p) => Syntax::for_file(self.lang(), Some(p)),
            None => Some(Syntax::Rust),
        }
    }

    pub fn rehighlight(&mut self, hl: &mut Highlighter) {
        self.classes = match self.syntax() {
            Some(syntax) if self.buffer.byte_len() <= MAX_HIGHLIGHT_BYTES => {
                hl.highlight(syntax, &self.buffer.text())
            }
            _ => Vec::new(),
        };
    }
    /// Replace the whole text (after the file changed on disk), keeping the cursor line.
    pub fn replace_text(&mut self, text: &str, hl: &mut Highlighter) {
        let (line, col) = self.buffer.cursor();
        self.buffer = Buffer::new(text);
        self.buffer.set_cursor(line, col, false);
        self.dirty = false;
        self.stamp = self.path.as_deref().and_then(disk_stamp);
        self.rehighlight(hl);
    }

    pub fn edited(&mut self, hl: &mut Highlighter) {
        self.dirty = true;
        self.rehighlight(hl);
    }

    pub fn save(&mut self) -> Result<PathBuf, String> {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => rfd::FileDialog::new()
                .save_file()
                .ok_or_else(|| "Save cancelled".to_string())?,
        };
        std::fs::write(&path, self.buffer.text()).map_err(|e| format!("Save failed: {e}"))?;
        self.stamp = disk_stamp(&path);
        self.path = Some(path.clone());
        self.dirty = false;
        Ok(path)
    }
}

/// Scroll offset of one pane's view of a document, in pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scroll {
    pub x: f32,
    pub y: f32,
}

impl Scroll {
    /// This offset limited to the document's extent, so a view stays valid
    /// after edits made through another pane.
    pub fn clamped(self, buf: &Buffer, vp: Size) -> Scroll {
        let max = (buf.line_count() as f32 * LINE_H - vp.height + LINE_H * 3.0).max(0.0);
        Scroll {
            x: self.x.max(0.0),
            y: self.y.clamp(0.0, max),
        }
    }

    pub fn ensure_cursor_visible(&mut self, buf: &Buffer, vp: Size) {
        let (line, col) = buf.cursor();
        let y = line as f32 * LINE_H;
        if y < self.y {
            self.y = y;
        } else if y + LINE_H > self.y + vp.height {
            self.y = y + LINE_H - vp.height;
        }
        let x = visual_col(&buf.line_text(line), col) as f32 * CHAR_W;
        let view_w = (vp.width - TEXT_X - 20.0).max(CHAR_W);
        if x < self.x {
            self.x = x;
        } else if x > self.x + view_w {
            self.x = x - view_w;
        }
        *self = self.clamped(buf, vp);
    }
}

pub fn visual_col(line: &str, col: usize) -> usize {
    line.chars().take(col).fold(0, |v, c| {
        if c == '\t' {
            v + TAB_WIDTH - v % TAB_WIDTH
        } else {
            v + 1
        }
    })
}

pub fn col_from_visual(line: &str, vcol: usize) -> usize {
    let mut v = 0;
    for (i, c) in line.chars().enumerate() {
        let w = if c == '\t' { TAB_WIDTH - v % TAB_WIDTH } else { 1 };
        if vcol <= v {
            return i;
        }
        if vcol < v + w {
            return if vcol - v <= w / 2 { i } else { i + 1 };
        }
        v += w;
    }
    line.chars().count()
}
