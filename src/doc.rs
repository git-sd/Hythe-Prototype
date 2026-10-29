use std::path::PathBuf;

use iced::Size;

use crate::buffer::Buffer;
use crate::highlight::RustHighlighter;

pub const FONT_SIZE: f32 = 15.0;
pub const LINE_H: f32 = 20.0;
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
}

impl Doc {
    pub fn empty() -> Self {
        Self::from_text("", None)
    }

    pub fn open(path: PathBuf, hl: &mut RustHighlighter) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(&path)?;
        let mut doc = Self::from_text(&text, Some(path));
        doc.rehighlight(hl);
        Ok(doc)
    }

    fn from_text(text: &str, path: Option<PathBuf>) -> Self {
        Self {
            buffer: Buffer::new(text),
            path,
            dirty: false,
            classes: Vec::new(),
        }
    }

    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into())
    }

    pub fn language(&self) -> &'static str {
        match self.path.as_ref().and_then(|p| p.extension()).and_then(|e| e.to_str()) {
            Some("rs") | None => "Rust",
            _ => "Plain Text",
        }
    }

    pub fn rehighlight(&mut self, hl: &mut RustHighlighter) {
        self.classes = if self.language() == "Rust" {
            hl.highlight(&self.buffer.text())
        } else {
            Vec::new()
        };
    }

    pub fn edited(&mut self, hl: &mut RustHighlighter) {
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
