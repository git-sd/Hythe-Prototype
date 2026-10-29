//! The completion popup: state, geometry and drawing.

use hythe_complete::{Item, Kind};
use iced::widget::canvas;
use iced::{Color, Font, Pixels, Point, Rectangle, Size};

use crate::doc::{CHAR_W, LINE_H};
use crate::theme;

const VISIBLE_ROWS: usize = 8;
const FONT: f32 = 14.0;
const PAD: f32 = 4.0;

pub struct CompletionUi {
    pub items: Vec<Item>,
    pub selected: usize,
    /// The user moved the selection with the arrow keys, so Enter accepts.
    pub navigated: bool,
    /// Character index where the typed prefix starts.
    pub start_char: usize,
}

impl CompletionUi {
    /// Index of the first visible row, keeping the selection in view.
    fn first_visible(&self) -> usize {
        (self.selected + 1).saturating_sub(VISIBLE_ROWS)
    }

    /// Where the popup sits for an anchor (top-left of the prefix, and the
    /// line height below it), flipped above the line if there's no room.
    pub fn rect(&self, anchor: Point, bounds_size: Size, line_h: f32) -> Rectangle {
        let rows = self.items.len().min(VISIBLE_ROWS);
        let chars = self
            .items
            .iter()
            .map(|i| 3 + i.label.chars().count() + 2 + i.detail.chars().count())
            .max()
            .unwrap_or(10);
        let width = (chars as f32 * CHAR_W).clamp(240.0, 560.0).min(bounds_size.width - 8.0);
        let height = rows as f32 * LINE_H + 2.0 * PAD;
        let mut x = anchor.x;
        if x + width > bounds_size.width {
            x = (bounds_size.width - width).max(0.0);
        }
        let below = anchor.y + line_h;
        let y = if below + height > bounds_size.height { (anchor.y - height).max(0.0) } else { below };
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    /// Item under a point inside `rect`, if any.
    pub fn item_at(&self, rect: Rectangle, p: Point) -> Option<usize> {
        if !rect.contains(p) {
            return None;
        }
        let row = ((p.y - rect.y - PAD) / LINE_H).floor();
        if row < 0.0 {
            return None;
        }
        let i = self.first_visible() + row as usize;
        (i < self.items.len() && (row as usize) < VISIBLE_ROWS).then_some(i)
    }

    pub fn draw(&self, frame: &mut canvas::Frame, rect: Rectangle) {
        frame.fill_rectangle(rect.position(), rect.size(), theme::BORDER);
        frame.fill_rectangle(
            Point::new(rect.x + 1.0, rect.y + 1.0),
            Size::new(rect.width - 2.0, rect.height - 2.0),
            theme::PANEL_BG,
        );

        let first = self.first_visible();
        for (row, (i, item)) in self.items.iter().enumerate().skip(first).take(VISIBLE_ROWS).enumerate() {
            let y = rect.y + PAD + row as f32 * LINE_H;
            if i == self.selected {
                frame.fill_rectangle(
                    Point::new(rect.x + 2.0, y),
                    Size::new(rect.width - 4.0, LINE_H),
                    theme::SELECTED,
                );
            }
            let text = |content: String, x: f32, color: Color, size: f32| canvas::Text {
                content,
                position: Point::new(x, y + 2.0),
                color,
                size: Pixels(size),
                font: Font::MONOSPACE,
                ..canvas::Text::default()
            };
            frame.fill_text(text(item.kind.tag().to_string(), rect.x + 8.0, kind_color(item.kind), FONT));
            frame.fill_text(text(item.label.clone(), rect.x + 8.0 + 2.2 * CHAR_W, theme::TEXT, FONT));
            if !item.detail.is_empty() {
                let w = item.detail.chars().count() as f32 * CHAR_W * 0.85;
                frame.fill_text(text(item.detail.clone(), rect.x + rect.width - w - 8.0, theme::TEXT_DIM, 12.0));
            }
        }
    }
}

fn kind_color(kind: Kind) -> Color {
    match kind {
        Kind::Function => Color::from_rgb8(0x6c, 0xb6, 0xff),
        Kind::Type => Color::from_rgb8(0xf0, 0xc6, 0x74),
        Kind::Variable => Color::from_rgb8(0x9a, 0xa3, 0xb2),
        Kind::Constant => Color::from_rgb8(0xf2, 0x8b, 0x6b),
        Kind::Module => Color::from_rgb8(0xe8, 0x7a, 0x8a),
        Kind::Keyword => Color::from_rgb8(0xc7, 0x92, 0xea),
        Kind::Snippet => theme::ACCENT,
        Kind::Builtin => Color::from_rgb8(0x7f, 0xd6, 0xd0),
        Kind::Member => Color::from_rgb8(0x7f, 0xd6, 0xd0),
    }
}
