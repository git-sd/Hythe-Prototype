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

/// One open file: its text, highlighting and scroll position.
pub struct Doc {
    pub buffer: Buffer,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub classes: Vec<u8>,
    pub scroll_x: f32,
    pub scroll_y: f32,
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
            scroll_x: 0.0,
            scroll_y: 0.0,
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

    pub fn ensure_cursor_visible(&mut self, vp: Size) {
        let (line, col) = self.buffer.cursor();
        let y = line as f32 * LINE_H;
        if y < self.scroll_y {
            self.scroll_y = y;
        } else if y + LINE_H > self.scroll_y + vp.height {
            self.scroll_y = y + LINE_H - vp.height;
        }
        let x = visual_col(&self.buffer.line_text(line), col) as f32 * CHAR_W;
        let view_w = (vp.width - TEXT_X - 20.0).max(CHAR_W);
        if x < self.scroll_x {
            self.scroll_x = x;
        } else if x > self.scroll_x + view_w {
            self.scroll_x = x - view_w;
        }
        self.clamp_scroll(vp);
    }

    pub fn clamp_scroll(&mut self, vp: Size) {
        let max = (self.buffer.line_count() as f32 * LINE_H - vp.height + LINE_H * 3.0).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
        self.scroll_x = self.scroll_x.max(0.0);
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
