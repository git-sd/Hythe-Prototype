use std::cell::Cell;

use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Event, Font, Pixels, Point, Rectangle, Renderer, Size, Theme};

use crate::doc::*;
use crate::highlight;
use crate::theme;
use crate::Message;

const SELECTION: Color = Color::from_rgba(0.25, 0.75, 0.69, 0.25);
const CURRENT_LINE: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.035);

fn class_color(class: u8) -> Color {
    let Some(name) = highlight::NAMES.get(class as usize) else {
        return theme::TEXT;
    };
    match *name {
        "comment" => Color::from_rgb8(0x5f, 0x69, 0x78),
        "string" | "escape" => Color::from_rgb8(0xa5, 0xc2, 0x61),
        "keyword" => Color::from_rgb8(0xc7, 0x92, 0xea),
        "function" | "function.builtin" | "function.macro" => Color::from_rgb8(0x6c, 0xb6, 0xff),
        "type" | "type.builtin" | "constructor" => Color::from_rgb8(0xf0, 0xc6, 0x74),
        "constant" | "constant.builtin" => Color::from_rgb8(0xf2, 0x8b, 0x6b),
        "attribute" | "label" => Color::from_rgb8(0xd9, 0xa4, 0x86),
        "property" => Color::from_rgb8(0x7f, 0xd6, 0xd0),
        "variable.builtin" | "variable.parameter" => Color::from_rgb8(0xe8, 0x7a, 0x8a),
        "operator" => Color::from_rgb8(0x7f, 0xd6, 0xd0),
        "punctuation" | "punctuation.bracket" | "punctuation.delimiter" => {
            Color::from_rgb8(0x9a, 0xa3, 0xb2)
        }
        _ => theme::TEXT,
    }
}

pub struct EditorView<'a> {
    pub doc: &'a Doc,
    pub scroll: Scroll,
    pub pane: usize,
    pub focused: bool,
    pub viewport: &'a Cell<Size>,
}

#[derive(Default)]
pub struct DragState {
    dragging: bool,
}

impl EditorView<'_> {
    fn hit(&self, p: Point) -> (usize, usize) {
        let doc = self.doc;
        let scroll = self.scroll.clamped(&doc.buffer, self.viewport.get());
        let line = (((p.y + scroll.y) / LINE_H).floor().max(0.0) as usize)
            .min(doc.buffer.line_count() - 1);
        let vcol = ((p.x - TEXT_X + scroll.x) / CHAR_W).round().max(0.0) as usize;
        (line, col_from_visual(&doc.buffer.line_text(line), vcol))
    }
}

impl canvas::Program<Message> for EditorView<'_> {
    type State = DragState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        self.viewport.set(bounds.size());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                state.dragging = true;
                let (line, col) = self.hit(p);
                let pane = self.pane;
                Some(canvas::Action::publish(Message::Click { pane, line, col, select: false }).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                let p = cursor.position_in(bounds)?;
                let (line, col) = self.hit(p);
                let pane = self.pane;
                Some(canvas::Action::publish(Message::Click { pane, line, col, select: true }).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                None
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * LINE_H * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                Some(canvas::Action::publish(Message::Scroll { pane: self.pane, dy }).and_capture())
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
        let doc = self.doc;
        let buf = &doc.buffer;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::EDITOR_BG);

        let scroll = self.scroll.clamped(buf, bounds.size());
        let first = (scroll.y / LINE_H).floor() as usize;
        let last = (first + (bounds.height / LINE_H).ceil() as usize + 1).min(buf.line_count());
        let (cur_line, cur_col) = buf.cursor();
        let text_x = TEXT_X - scroll.x;

        for line in first..last {
            let y = line as f32 * LINE_H - scroll.y;
            let text = buf.line_text(line);
            let len = text.chars().count();

            if line == cur_line && self.focused {
                frame.fill_rectangle(Point::new(0.0, y), Size::new(bounds.width, LINE_H), CURRENT_LINE);
            }
            if let Some((s, e)) = buf.selection_on_line(line) {
                let x0 = text_x + visual_col(&text, s) as f32 * CHAR_W;
                let x1 = text_x
                    + visual_col(&text, e.min(len)) as f32 * CHAR_W
                    + if e > len { CHAR_W / 2.0 } else { 0.0 };
                frame.fill_rectangle(Point::new(x0, y), Size::new((x1 - x0).max(2.0), LINE_H), SELECTION);
            }

            // Colored runs, split where the highlight class changes.
            let mut byte = buf.line_start_byte(line);
            let mut vcol = 0usize;
            let mut run = String::new();
            let mut run_vcol = 0usize;
            let mut run_class = highlight::NONE;
            let flush = |frame: &mut canvas::Frame, run: &mut String, run_vcol: usize, class: u8| {
                if run.trim().is_empty() {
                    run.clear();
                    return;
                }
                frame.fill_text(canvas::Text {
                    content: std::mem::take(run),
                    position: Point::new(text_x + run_vcol as f32 * CHAR_W, y + 2.0),
                    color: class_color(class),
                    size: Pixels(FONT_SIZE),
                    font: Font::MONOSPACE,
                    ..canvas::Text::default()
                });
            };
            for c in text.chars() {
                let class = doc.classes.get(byte).copied().unwrap_or(highlight::NONE);
                let w = if c == '\t' { TAB_WIDTH - vcol % TAB_WIDTH } else { 1 };
                if class != run_class && !run.is_empty() {
                    flush(&mut frame, &mut run, run_vcol, run_class);
                }
                if run.is_empty() {
                    run_vcol = vcol;
                    run_class = class;
                }
                if c == '\t' {
                    run.push_str(&" ".repeat(w));
                } else {
                    run.push(c);
                }
                vcol += w;
                byte += c.len_utf8();
            }
            flush(&mut frame, &mut run, run_vcol, run_class);

            frame.fill_text(canvas::Text {
                content: format!("{:>width$}", line + 1, width = GUTTER_COLS - 1),
                position: Point::new(4.0, y + 2.0),
                color: if line == cur_line { theme::TEXT } else { theme::TEXT_DIM },
                size: Pixels(FONT_SIZE),
                font: Font::MONOSPACE,
                ..canvas::Text::default()
            });
        }

        let cy = cur_line as f32 * LINE_H - scroll.y;
        let cx = text_x + visual_col(&buf.line_text(cur_line), cur_col) as f32 * CHAR_W;
        frame.fill_rectangle(Point::new(cx, cy + 1.0), Size::new(2.0, LINE_H - 2.0),
            if self.focused { theme::ACCENT } else { theme::TEXT_DIM },
        );

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
