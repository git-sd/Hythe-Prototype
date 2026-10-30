//! Draws a `hythe_term::Snapshot` and turns mouse input into messages.

use std::sync::Arc;

use hythe_term::{style, CursorKind, Rgb, Session, Snapshot};
use iced::font::{Style as FontStyle, Weight};
use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Event, Font, Pixels, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;
use crate::Message;

pub const FONT_SIZE: f32 = 14.0;
pub const CELL_W: f32 = FONT_SIZE * 0.6;
pub const CELL_H: f32 = 18.0;
pub const PAD: f32 = 8.0;

/// Whole cells that fit in `size`.
pub fn grid_for(size: Size) -> (u16, u16) {
    let cols = ((size.width - 2.0 * PAD) / CELL_W).floor().max(2.0);
    let rows = ((size.height - 2.0 * PAD) / CELL_H).floor().max(1.0);
    (cols as u16, rows as u16)
}

fn color(c: Rgb) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}

pub struct TerminalView<'a> {
    pub snap: &'a Snapshot,
    pub session: Arc<Session>,
    pub focused: bool,
}

#[derive(Default)]
pub struct DragState {
    dragging: bool,
}

impl TerminalView<'_> {
    fn cell_at(&self, p: Point) -> (usize, usize) {
        let col = ((p.x - PAD) / CELL_W).floor().max(0.0) as usize;
        let row = ((p.y - PAD) / CELL_H).floor().max(0.0) as usize;
        (
            row.min(self.snap.rows.saturating_sub(1)),
            col.min(self.snap.cols.saturating_sub(1)),
        )
    }
}

impl canvas::Program<Message> for TerminalView<'_> {
    type State = DragState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                state.dragging = true;
                let (row, col) = self.cell_at(p);
                Some(canvas::Action::publish(Message::TermPress { row, col }).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                // Keep extending the selection even when the pointer leaves the panel.
                let p = cursor.position_from(bounds.position())?;
                let (row, col) = self.cell_at(p);
                Some(canvas::Action::publish(Message::TermDrag { row, col }).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                None
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => (y * 3.0).round() as i32,
                    mouse::ScrollDelta::Pixels { y, .. } => (y / CELL_H).round() as i32,
                };
                (lines != 0).then(|| canvas::Action::publish(Message::TermScroll(lines)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // Keep the pty the same size as the panel; the next tick picks it up.
        let (cols, rows) = grid_for(bounds.size());
        if cols as usize != self.snap.cols || rows as usize != self.snap.rows {
            self.session.resize(cols, rows, CELL_W as u16, CELL_H as u16);
        }

        let snap = self.snap;
        let default_bg = snap.background;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), color(default_bg));

        let selection_bg = Color::from_rgba(0.25, 0.75, 0.69, 0.35);
        let mono = |bold: bool, italic: bool| Font {
            weight: if bold { Weight::Bold } else { Weight::Normal },
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
            ..crate::theme::MONO
        };

        for row in 0..snap.rows {
            let y = PAD + row as f32 * CELL_H;
            let cells = snap.row(row);

            // Backgrounds: merge neighbouring cells with the same colour.
            let mut col = 0;
            while col < snap.cols {
                let c = &cells[col];
                let selected = c.has(style::SELECTED);
                if c.bg == default_bg && !selected {
                    col += 1;
                    continue;
                }
                let mut end = col + 1;
                while end < snap.cols
                    && cells[end].bg == c.bg
                    && cells[end].has(style::SELECTED) == selected
                {
                    end += 1;
                }
                frame.fill_rectangle(
                    Point::new(PAD + col as f32 * CELL_W, y),
                    Size::new((end - col) as f32 * CELL_W, CELL_H),
                    if selected { selection_bg } else { color(c.bg) },
                );
                col = end;
            }

            // Text: merge runs that share colour and weight; blank cells break runs.
            let mut col = 0;
            while col < snap.cols {
                let c = &cells[col];
                if c.ch == ' ' || c.has(style::SPACER) {
                    col += 1;
                    continue;
                }
                let key = (c.fg, c.style & (style::BOLD | style::ITALIC));
                let mut text = String::new();
                let mut end = col;
                while end < snap.cols {
                    let n = &cells[end];
                    if n.has(style::SPACER) {
                        end += 1;
                        continue;
                    }
                    if n.ch == ' ' || (n.fg, n.style & (style::BOLD | style::ITALIC)) != key || n.has(style::WIDE) != c.has(style::WIDE) {
                        break;
                    }
                    text.push(n.ch);
                    end += 1;
                }
                let x = PAD + col as f32 * CELL_W;
                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(x, y + 1.0),
                    color: color(c.fg),
                    size: Pixels(FONT_SIZE),
                    font: mono(c.has(style::BOLD), c.has(style::ITALIC)),
                    ..canvas::Text::default()
                });
                // Underline/strike drawn per run.
                if c.has(style::UNDERLINE) {
                    frame.fill_rectangle(
                        Point::new(x, y + CELL_H - 3.0),
                        Size::new((end - col) as f32 * CELL_W, 1.0),
                        color(c.fg),
                    );
                }
                if c.has(style::STRIKE) {
                    frame.fill_rectangle(
                        Point::new(x, y + CELL_H / 2.0),
                        Size::new((end - col) as f32 * CELL_W, 1.0),
                        color(c.fg),
                    );
                }
                col = end.max(col + 1);
            }
        }

        if let Some(cur) = snap.cursor {
            let x = PAD + cur.col as f32 * CELL_W;
            let y = PAD + cur.row as f32 * CELL_H;
            let accent = theme::ACCENT;
            match (cur.kind, self.focused) {
                (CursorKind::Block, true) => frame.fill_rectangle(
                    Point::new(x, y),
                    Size::new(CELL_W, CELL_H),
                    Color { a: 0.7, ..accent },
                ),
                (CursorKind::Block, false) => frame.stroke_rectangle(
                    Point::new(x + 0.5, y + 0.5),
                    Size::new(CELL_W - 1.0, CELL_H - 1.0),
                    canvas::Stroke::default().with_color(theme::TEXT_DIM).with_width(1.0),
                ),
                (CursorKind::Underline, _) => frame.fill_rectangle(
                    Point::new(x, y + CELL_H - 2.0),
                    Size::new(CELL_W, 2.0),
                    accent,
                ),
                (CursorKind::Beam, _) => {
                    frame.fill_rectangle(Point::new(x, y), Size::new(2.0, CELL_H), accent)
                }
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _: &Self::State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}
