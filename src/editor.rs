use std::cell::Cell;

use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Event, Font, Pixels, Point, Rectangle, Renderer, Size, Theme};

use crate::completion_ui::CompletionUi;
use crate::doc::*;
use crate::highlight;
use crate::theme;
use crate::Message;

const SELECTION: Color = Color::from_rgba(0.45, 0.68, 0.91, 0.25);
const CURRENT_LINE: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.035);

pub(crate) fn class_color(class: u8) -> Color {
    let Some(name) = highlight::NAMES.get(class as usize) else {
        return theme::TEXT;
    };
    match *name {
        "comment" => Color::from_rgb8(0x5d, 0x63, 0x6f),
        "string" | "string.escape" | "string.regexp" | "string.special" | "escape" => {
            Color::from_rgb8(0xa1, 0xc1, 0x81)
        }
        "keyword" => Color::from_rgb8(0xb4, 0x77, 0xcf),
        "function" | "function.builtin" | "function.macro" | "constructor" => {
            Color::from_rgb8(0x73, 0xad, 0xe9)
        }
        "type" | "type.builtin" | "module" | "namespace" => Color::from_rgb8(0x6e, 0xb4, 0xbf),
        "constant" | "constant.builtin" | "number" | "boolean" => Color::from_rgb8(0xbf, 0x95, 0x6a),
        "attribute" | "label" | "tag" => Color::from_rgb8(0x74, 0xad, 0xe8),
        "property" => Color::from_rgb8(0xd0, 0x72, 0x77),
        "variable.builtin" | "variable.parameter" => Color::from_rgb8(0xbf, 0x95, 0x6a),
        "operator" => Color::from_rgb8(0x6e, 0xb4, 0xbf),
        "punctuation" | "punctuation.bracket" | "punctuation.delimiter" | "punctuation.special"
        | "tag.delimiter" => Color::from_rgb8(0xac, 0xb2, 0xbe),        _ => theme::TEXT,
    }
}

pub struct EditorView<'a> {
    pub doc: &'a Doc,
    pub scroll: Scroll,
    pub pane: usize,
    pub focused: bool,
    pub viewport: &'a Cell<Size>,
    /// Completion popup, when this pane has one open.
    pub popup: Option<&'a CompletionUi>,
    /// Inline blame note for the cursor line.
    pub blame: Option<String>,
    /// Collaborators' carets in this document.
    pub remotes: Vec<crate::collab::RemoteCursor>,
}

#[derive(Default)]
pub struct DragState {
    dragging: bool,
}

impl EditorView<'_> {
    /// Popup rectangle in canvas coordinates, anchored under the typed prefix.
    fn popup_rect(&self, ui: &CompletionUi, bounds_size: Size) -> Rectangle {
        let m = self.scroll.metrics();
        let buf = &self.doc.buffer;
        let scroll = self.scroll.clamped(buf, bounds_size);
        let (line, col) = buf.line_col(ui.start_char);
        let x = m.text_x - scroll.x + visual_col(&buf.line_text(line), col) as f32 * m.char_w;
        let y = line as f32 * m.line_h - scroll.y;
        ui.rect(Point::new(x, y), bounds_size, m.line_h)
    }

    fn hit(&self, p: Point) -> (usize, usize) {
        let m = self.scroll.metrics();
        let doc = self.doc;
        let scroll = self.scroll.clamped(&doc.buffer, self.viewport.get());
        let line = (((p.y + scroll.y) / m.line_h).floor().max(0.0) as usize)
            .min(doc.buffer.line_count() - 1);
        let vcol = ((p.x - m.text_x + scroll.x) / m.char_w).round().max(0.0) as usize;
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
        let m = self.scroll.metrics();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                // A click on the completion popup picks that entry.
                if let Some(ui) = self.popup {
                    let rect = self.popup_rect(ui, bounds.size());
                    if let Some(i) = ui.item_at(rect, p) {
                        return Some(canvas::Action::publish(Message::CompletionPick(i)).and_capture());
                    }
                }
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
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * m.char_w * 6.0, y * m.line_h * 3.0),
                    mouse::ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                Some(canvas::Action::publish(Message::Scroll { pane: self.pane, dy, dx }).and_capture())
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
        let m = self.scroll.metrics();
        let doc = self.doc;
        let buf = &doc.buffer;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::EDITOR_BG);

        let scroll = self.scroll.clamped(buf, bounds.size());
        let first = (scroll.y / m.line_h).floor() as usize;
        let last = (first + (bounds.height / m.line_h).ceil() as usize + 1).min(buf.line_count());
        let (cur_line, cur_col) = buf.cursor();
        let text_x = m.text_x - scroll.x;

        for line in first..last {
            let y = line as f32 * m.line_h - scroll.y;
            let text = buf.line_text(line);
            let len = text.chars().count();

            if line == cur_line && self.focused {
                frame.fill_rectangle(Point::new(0.0, y), Size::new(bounds.width, m.line_h), CURRENT_LINE);
            }
            if let Some((s, e)) = buf.selection_on_line(line) {
                let x0 = text_x + visual_col(&text, s) as f32 * m.char_w;
                let x1 = text_x
                    + visual_col(&text, e.min(len)) as f32 * m.char_w
                    + if e > len { m.char_w / 2.0 } else { 0.0 };
                frame.fill_rectangle(Point::new(x0, y), Size::new((x1 - x0).max(2.0), m.line_h), SELECTION);
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
                    position: Point::new(text_x + run_vcol as f32 * m.char_w, y + 2.0),
                    color: class_color(class),
                    size: Pixels(m.font),
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
                color: if line == cur_line { theme::TEXT } else { theme::GUTTER },
                size: Pixels(m.font),
                font: Font::MONOSPACE,
                ..canvas::Text::default()
            });
        }

        let cy = cur_line as f32 * m.line_h - scroll.y;
        let cx = text_x + visual_col(&buf.line_text(cur_line), cur_col) as f32 * m.char_w;
        frame.fill_rectangle(Point::new(cx, cy + 1.0), Size::new(2.0, m.line_h - 2.0),
            if self.focused { theme::ACCENT } else { theme::TEXT_DIM },
        );

        if let Some(note) = &self.blame {
            let line_len = buf.line_text(cur_line).chars().count();
            let x = text_x + visual_col(&buf.line_text(cur_line), line_len) as f32 * m.char_w + 3.0 * m.char_w;
            if cur_line >= first && cur_line < last {
                frame.fill_text(canvas::Text {
                    content: note.clone(),
                    position: Point::new(x, cur_line as f32 * m.line_h - scroll.y + 3.0),
                    color: theme::TEXT_FAINT,
                    size: Pixels(m.font - 1.5),
                    font: Font { style: iced::font::Style::Italic, ..Font::MONOSPACE },
                    ..canvas::Text::default()
                });
            }
        }
        for r in &self.remotes {
            let (line, col) = buf.line_col(r.head);
            if line < first || line >= last {
                continue;
            }
            let x = text_x + visual_col(&buf.line_text(line), col) as f32 * m.char_w;
            let y = line as f32 * m.line_h - scroll.y;
            frame.fill_rectangle(Point::new(x, y + 1.0), Size::new(2.0, m.line_h - 2.0), r.color);
            let tag_w = r.name.chars().count() as f32 * 6.4 + 8.0;
            let tag_y = if y < 14.0 { y + m.line_h - 2.0 } else { y - 13.0 };
            frame.fill_rectangle(Point::new(x, tag_y), Size::new(tag_w, 13.0), r.color);
            frame.fill_text(canvas::Text {
                content: r.name.clone(),
                position: Point::new(x + 4.0, tag_y + 1.0),
                color: Color::from_rgb8(0x1b, 0x1e, 0x24),
                size: Pixels(10.0),
                font: Font::MONOSPACE,
                ..canvas::Text::default()
            });
        }
        if let Some(ui) = self.popup {
            let rect = self.popup_rect(ui, bounds.size());
            ui.draw(&mut frame, rect);
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
