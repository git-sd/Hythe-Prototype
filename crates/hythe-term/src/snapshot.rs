use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

use crate::palette::{Palette, Rgb};

/// Bits in [`Cell::style`].
pub mod style {
    pub const BOLD: u8 = 1;
    pub const ITALIC: u8 = 2;
    pub const UNDERLINE: u8 = 4;
    pub const STRIKE: u8 = 8;
    /// Double-width character; the next cell is a [`SPACER`].
    pub const WIDE: u8 = 16;
    /// Right half of a double-width character; draw nothing.
    pub const SPACER: u8 = 32;
    /// Inside the current selection.
    pub const SELECTED: u8 = 64;
}

/// One resolved terminal cell. Colours already account for inverse video and dimming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub style: u8,
}

impl Cell {
    pub fn has(&self, bit: u8) -> bool {
        self.style & bit != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Underline,
    Beam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub kind: CursorKind,
}

/// An immutable copy of what a terminal currently shows.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    /// Row-major, `rows * cols` cells.
    pub cells: Vec<Cell>,
    /// `None` when the application hid the cursor or the view is scrolled back.
    pub cursor: Option<Cursor>,
    /// Lines the view is scrolled up from the bottom (0 = live).
    pub scrollback_offset: usize,
    /// True when the application asked to be told about mouse events.
    pub mouse_reporting: bool,
    /// True on the alternate screen (full-screen apps such as editors and agent UIs).
    pub alt_screen: bool,
    pub title: String,
    /// Increases whenever the terminal output changes.
    pub generation: u64,
    pub background: Rgb,
}

impl Snapshot {
    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * self.cols + col]
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        &self.cells[row * self.cols..(row + 1) * self.cols]
    }

    /// Text of one row with trailing blanks removed.
    pub fn line_text(&self, row: usize) -> String {
        let mut s: String = self
            .row(row)
            .iter()
            .filter(|c| !c.has(style::SPACER))
            .map(|c| c.ch)
            .collect();
        s.truncate(s.trim_end().len());
        s
    }

    /// All rows joined with newlines, trailing blank rows dropped.
    pub fn text(&self) -> String {
        let mut lines: Vec<String> = (0..self.rows).map(|r| self.line_text(r)).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}

fn rgb(c: alacritty_terminal::vte::ansi::Rgb) -> Rgb {
    Rgb(c.r, c.g, c.b)
}

fn resolve(color: Color, colors: &Colors, pal: &Palette) -> Rgb {
    match color {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => match colors[i as usize] {
            Some(c) => rgb(c),
            None => pal.indexed(i),
        },
        Color::Named(n) => {
            if let Some(c) = colors[n] {
                return rgb(c);
            }
            let idx = n as usize;
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => pal.fg,
                NamedColor::DimForeground => pal.dim(pal.fg),
                NamedColor::Background => pal.bg,
                NamedColor::Cursor => pal.cursor,
                _ if idx < 16 => pal.ansi[idx],
                _ => {
                    // Dim variants of the eight base colours.
                    let base = idx - NamedColor::DimBlack as usize;
                    pal.dim(pal.ansi[base.min(7)])
                }
            }
        }
    }
}

pub(crate) fn build<T: EventListener>(
    term: &Term<T>,
    pal: &Palette,
    generation: u64,
    title: String,
) -> Snapshot {
    let rows = term.screen_lines();
    let cols = term.columns();
    let blank = Cell { ch: ' ', fg: pal.fg, bg: pal.bg, style: 0 };
    let mut cells = vec![blank; rows * cols];

    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        let col = indexed.point.column.0;
        if row < 0 || row as usize >= rows || col >= cols {
            continue;
        }
        let c = indexed.cell;
        let mut fg = resolve(c.fg, content.colors, pal);
        let mut bg = resolve(c.bg, content.colors, pal);
        if c.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if c.flags.intersects(Flags::DIM) {
            fg = pal.dim(fg);
        }

        let mut st = 0u8;
        let f = c.flags;
        if f.contains(Flags::BOLD) {
            st |= style::BOLD;
        }
        if f.contains(Flags::ITALIC) {
            st |= style::ITALIC;
        }
        if f.intersects(Flags::ALL_UNDERLINES) {
            st |= style::UNDERLINE;
        }
        if f.contains(Flags::STRIKEOUT) {
            st |= style::STRIKE;
        }
        if f.contains(Flags::WIDE_CHAR) {
            st |= style::WIDE;
        }
        let mut ch = c.c;
        if f.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            st |= style::SPACER;
            ch = ' ';
        }
        if f.contains(Flags::HIDDEN) {
            ch = ' ';
        }
        if let Some(sel) = &content.selection {
            if sel.contains(Point::new(indexed.point.line, Column(col))) {
                st |= style::SELECTED;
            }
        }
        cells[row as usize * cols + col] = Cell { ch, fg, bg, style: st };
    }

    let cur = content.cursor;
    let cursor_row = cur.point.line.0 + offset;
    let shown = content.mode.contains(TermMode::SHOW_CURSOR)
        && cur.shape != CursorShape::Hidden
        && (0..rows as i32).contains(&cursor_row)
        && cur.point.column.0 < cols;
    let cursor = shown.then(|| Cursor {
        row: cursor_row as usize,
        col: cur.point.column.0,
        kind: match cur.shape {
            CursorShape::Underline => CursorKind::Underline,
            CursorShape::Beam => CursorKind::Beam,
            _ => CursorKind::Block,
        },
    });

    Snapshot {
        cols,
        rows,
        cells,
        cursor,
        scrollback_offset: content.display_offset,
        mouse_reporting: content.mode.intersects(TermMode::MOUSE_MODE),
        alt_screen: content.mode.contains(TermMode::ALT_SCREEN),
        title,
        generation,
        background: pal.bg,
    }
}
