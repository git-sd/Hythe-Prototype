mod buffer;
mod doc;
mod editor;
mod explorer;
mod highlight;
mod palette;
mod panes;
mod theme;

use std::path::PathBuf;

use iced::keyboard::{self, key::Named, Key};
use iced::widget::canvas::Canvas;
use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, space, stack, text};
use iced::{clipboard, window, Alignment, Element, Length, Subscription, Task, Theme};

use doc::{Doc, TAB_WIDTH};
use editor::EditorView;
use explorer::Node;
use highlight::RustHighlighter;
use palette::Palette;
use panes::{Axis, Panes};

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Dark)
        .window_size((1200.0, 760.0))
        .exit_on_close_request(false)
        .run()
}

enum Unsaved {
    Save,
    Discard,
    Cancel,
}

fn ask_unsaved(body: &str, save_label: &str) -> Unsaved {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult as R, MessageLevel};
    const DISCARD: &str = "Don't Save";
    let result = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Unsaved changes")
        .set_description(body)
        .set_buttons(MessageButtons::YesNoCancelCustom(
            save_label.into(),
            DISCARD.into(),
            "Cancel".into(),
        ))
        .show();
    match result {
        R::Yes => Unsaved::Save,
        R::No => Unsaved::Discard,
        R::Custom(s) if s == save_label => Unsaved::Save,
        R::Custom(s) if s == DISCARD => Unsaved::Discard,
        _ => Unsaved::Cancel,
    }
}

struct App {
    docs: Vec<Doc>,
    panes: Panes,
    root: Option<Node>,
    show_sidebar: bool,
    show_ai: bool,
    status: String,
    palette: Option<Palette>,
    highlighter: RustHighlighter,
}

#[derive(Debug, Clone)]
pub enum Message {
    Key(keyboard::Event),
    Click { pane: usize, line: usize, col: usize, select: bool },
    Scroll { pane: usize, dy: f32 },
    Paste(Option<String>),
    NewFile,
    OpenFileDialog,
    OpenFolderDialog,
    OpenFile(PathBuf),
    ToggleDir(PathBuf),
    SelectTab { pane: usize, tab: usize },
    CloseTab { pane: usize, tab: usize },
    Split(Axis),
    ClosePane,
    FocusNextPane,
    FocusPane(usize),
    CloseRequested,
    ToggleSidebar,
    ToggleAi,
    Save,
    CloseActive,
    Undo,
    Redo,
    SelectAll,
    OpenPalette,
    ClosePalette,
    RunPalette(usize),
}

impl App {
    fn new() -> Self {
        let mut app = App {
            docs: Vec::new(),
            panes: Panes::new(),
            root: None,
            show_sidebar: true,
            show_ai: true,
            status: String::new(),
            palette: None,
            highlighter: RustHighlighter::new(),
        };
        if let Some(arg) = std::env::args().nth(1) {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                app.root = Some(Node::root(path));
            } else {
                app.open_file(path);
            }
        }
        app
    }

    fn title(&self) -> String {
        match self.docs.get(self.panes.cur()) {
            Some(d) => format!("{}{} — Hythe", d.name(), if d.dirty { " •" } else { "" }),
            None => "Hythe".into(),
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        if let Some(i) = self.docs.iter().position(|d| d.path.as_ref() == Some(&path)) {
            self.panes.open_doc(i);
            return;
        }
        match Doc::open(path.clone(), &mut self.highlighter) {
            Ok(doc) => {
                if self.root.is_none() {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        self.root = Some(Node::root(parent.to_path_buf()));
                    }
                }
                self.docs.push(doc);
                self.panes.open_doc(self.docs.len() - 1);
                self.status = format!("Opened {}", path.display());
            }
            Err(e) => self.status = format!("Open failed: {e}"),
        }
    }

    fn close_tab(&mut self, pane: usize, tab: usize) {
        let Some(doc) = self.panes.list.get(pane).and_then(|p| p.tabs.get(tab)).map(|t| t.doc) else {
            return;
        };
        // Only the last view of a document can lose its edits.
        if self.panes.refs(doc) == 1 && !self.confirm_discard(&[doc]) {
            return;
        }
        if let Some(doc) = self.panes.close_tab(pane, tab) {
            self.prune_docs(vec![doc]);
        }
    }

    /// Ask what to do with unsaved edits in `docs`. Returns false if the
    /// user cancelled (or a save failed), meaning the close should not happen.
    fn confirm_discard(&mut self, docs: &[usize]) -> bool {
        let dirty: Vec<usize> = docs
            .iter()
            .copied()
            .filter(|&d| self.docs.get(d).is_some_and(|doc| doc.dirty))
            .collect();
        if dirty.is_empty() {
            return true;
        }
        let names: Vec<String> = dirty.iter().map(|&d| self.docs[d].name()).collect();
        let (body, save_label) = if names.len() == 1 {
            (format!("Do you want to save the changes to {}?", names[0]), "Save")
        } else {
            (
                format!(
                    "{} files have unsaved changes:\n{}\n\nYes saves all of them, No discards the changes.",
                    names.len(),
                    names.join("\n")
                ),
                "Save All",
            )
        };
        match ask_unsaved(&body, save_label) {
            Unsaved::Discard => true,
            Unsaved::Cancel => false,
            Unsaved::Save => {
                for d in dirty {
                    if let Err(e) = self.docs[d].save() {
                        self.status = e;
                        return false;
                    }
                }
                true
            }
        }
    }

    /// Drop documents that no pane shows any more.
    fn prune_docs(&mut self, mut candidates: Vec<usize>) {
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.dedup();
        for d in candidates {
            if !self.panes.uses_doc(d) {
                self.docs.remove(d);
                self.panes.forget_doc(d);
            }
        }
    }

    /// Scroll the focused pane so the cursor is on screen.
    fn reveal_cursor(&mut self) {
        let Some(pane) = self.panes.list.get_mut(self.panes.focus) else {
            return;
        };
        let vp = pane.viewport.get();
        if let Some(tab) = pane.tabs.get_mut(pane.active) {
            if let Some(d) = self.docs.get(tab.doc) {
                tab.scroll.ensure_cursor_visible(&d.buffer, vp);
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Scroll { pane, dy } => {
                if let Some(p) = self.panes.list.get_mut(pane) {
                    let vp = p.viewport.get();
                    if let Some(tab) = p.tabs.get_mut(p.active) {
                        if let Some(d) = self.docs.get(tab.doc) {
                            tab.scroll.y -= dy;
                            tab.scroll = tab.scroll.clamped(&d.buffer, vp);
                        }
                    }
                }
            }
            Message::Click { pane, line, col, select } => {
                self.panes.focus_pane(pane);
                if let Some(d) = self.docs.get_mut(self.panes.cur()) {
                    d.buffer.set_cursor(line, col, select);
                }
            }
            Message::Paste(Some(text)) => {
                if let Some(d) = self.docs.get_mut(self.panes.cur()) {
                    d.buffer.insert(&text.replace("\r\n", "\n"));
                    d.edited(&mut self.highlighter);
                }
                self.reveal_cursor();
            }
            Message::Paste(None) => {}
            Message::Key(keyboard::Event::KeyPressed { key, modifiers, text, .. }) => {
                if self.palette.is_some() {
                    return self.palette_key(key, modifiers, text.as_deref());
                }
                let task = self.handle_key(key, modifiers, text.as_deref());
                self.reveal_cursor();
                return task;
            }
            Message::Key(_) => {}
            Message::NewFile => {
                self.docs.push(Doc::empty());
                self.panes.open_doc(self.docs.len() - 1);
            }
            Message::OpenFileDialog => {
                if let Some(p) = rfd::FileDialog::new().pick_file() {
                    self.open_file(p);
                }
            }
            Message::OpenFolderDialog => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.root = Some(Node::root(p));
                    self.show_sidebar = true;
                }
            }
            Message::OpenFile(p) => self.open_file(p),
            Message::ToggleDir(p) => {
                if let Some(root) = &mut self.root {
                    root.toggle(&p);
                }
            }
            Message::SelectTab { pane, tab } => {
                self.panes.focus_pane(pane);
                if let Some(p) = self.panes.list.get_mut(pane) {
                    if tab < p.tabs.len() {
                        p.active = tab;
                    }
                }
            }
            Message::CloseTab { pane, tab } => self.close_tab(pane, tab),
            Message::Split(axis) => self.panes.split(axis),
            Message::ClosePane => {
                let focus = self.panes.focus;
                let at_risk: Vec<usize> = self.panes.list[focus]
                    .tabs
                    .iter()
                    .map(|t| t.doc)
                    .filter(|&d| self.panes.refs_outside(d, focus) == 0)
                    .collect();
                if self.panes.list.len() > 1 && self.confirm_discard(&at_risk) {
                    let docs = self.panes.close_pane(focus);
                    self.prune_docs(docs);
                }
            }
            Message::CloseRequested => {
                let all: Vec<usize> = (0..self.docs.len()).collect();
                if self.confirm_discard(&all) {
                    return iced::exit();
                }
            }
            Message::FocusNextPane => self.panes.focus_next(),
            Message::FocusPane(i) => self.panes.focus_pane(i),
            Message::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Message::ToggleAi => self.show_ai = !self.show_ai,
            Message::Save => {
                if let Some(d) = self.docs.get_mut(self.panes.cur()) {
                    self.status = match d.save() {
                        Ok(p) => format!("Saved {}", p.display()),
                        Err(e) => e,
                    };
                }
            }
            Message::CloseActive => {
                let p = &self.panes.list[self.panes.focus];
                let (pane, tab) = (self.panes.focus, p.active);
                self.close_tab(pane, tab);
            }
            Message::Undo | Message::Redo => {
                let undo = matches!(message, Message::Undo);
                if let Some(d) = self.docs.get_mut(self.panes.cur()) {
                    let changed = if undo { d.buffer.undo() } else { d.buffer.redo() };
                    if changed {
                        d.edited(&mut self.highlighter);
                    }
                }
                self.reveal_cursor();
            }
            Message::SelectAll => {
                if let Some(d) = self.docs.get_mut(self.panes.cur()) {
                    d.buffer.select_all();
                }
            }
            Message::OpenPalette => self.palette = Some(Palette::new()),
            Message::ClosePalette => self.palette = None,
            Message::RunPalette(i) => {
                let query = self.palette.take().map(|p| p.query).unwrap_or_default();
                if let Some(cmd) = palette::filter(&query).into_iter().nth(i) {
                    return self.update(cmd.message);
                }
            }
        }
        Task::none()
    }

    fn palette_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<&str>) -> Task<Message> {
        let Some(p) = self.palette.as_mut() else {
            return Task::none();
        };
        let count = palette::filter(&p.query).len();
        match key.as_ref() {
            Key::Named(Named::Escape) => self.palette = None,
            Key::Named(Named::Enter) => {
                let i = p.selected;
                return self.update(Message::RunPalette(i));
            }
            Key::Named(Named::ArrowDown) => p.selected = (p.selected + 1).min(count.saturating_sub(1)),
            Key::Named(Named::ArrowUp) => p.selected = p.selected.saturating_sub(1),
            Key::Named(Named::Backspace) => {
                p.query.pop();
                p.selected = 0;
            }
            Key::Character(c) if mods.command() && mods.shift() && c.eq_ignore_ascii_case("p") => {
                self.palette = None;
            }
            _ => {
                if !mods.command() {
                    let t: String = text.unwrap_or("").chars().filter(|c| !c.is_control()).collect();
                    if !t.is_empty() {
                        p.query.push_str(&t);
                        p.selected = 0;
                    }
                }
            }
        }
        Task::none()
    }

    fn handle_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<&str>) -> Task<Message> {
        let shift = mods.shift();
        let ctrl = mods.command();

        if ctrl {
            if let Key::Character(c) = key.as_ref() {
                let c = c.to_lowercase();
                match (c.as_str(), shift) {
                    ("n", _) => return self.update(Message::NewFile),
                    ("o", false) => return self.update(Message::OpenFileDialog),
                    ("o", true) => return self.update(Message::OpenFolderDialog),
                    ("p", true) => return self.update(Message::OpenPalette),
                    ("b", _) => return self.update(Message::ToggleSidebar),
                    ("\\", false) => return self.update(Message::Split(Axis::Horizontal)),
                    ("\\", true) => return self.update(Message::Split(Axis::Vertical)),
                    (n, false) if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0" => {
                        return self.update(Message::FocusPane((n.as_bytes()[0] - b'1') as usize))
                    }
                    ("w", _) => return self.update(Message::CloseActive),
                    ("s", _) => return self.update(Message::Save),
                    ("a", _) => return self.update(Message::SelectAll),
                    ("z", false) | ("y", _) => {
                        return self.update(if c == "z" { Message::Undo } else { Message::Redo })
                    }
                    ("z", true) => return self.update(Message::Redo),
                    ("c", _) => {
                        if let Some(t) = self.docs.get(self.panes.cur()).and_then(|d| d.buffer.selected_text()) {
                            return clipboard::write(t);
                        }
                    }
                    ("x", _) => {
                        if let Some(d) = self.docs.get_mut(self.panes.cur()) {
                            if let Some(t) = d.buffer.selected_text() {
                                d.buffer.delete_selection();
                                d.edited(&mut self.highlighter);
                                return clipboard::write(t);
                            }
                        }
                    }
                    ("v", _) => return clipboard::read().map(Message::Paste),
                    _ => {}
                }
                return Task::none();
            }
        }

        let vp = self.panes.list[self.panes.focus].viewport.get();
        let Some(d) = self.docs.get_mut(self.panes.cur()) else {
            return Task::none();
        };
        let hl = &mut self.highlighter;

        if let Key::Named(named) = key.as_ref() {
            let page = (vp.height / doc::LINE_H) as isize - 1;
            match named {
                Named::ArrowLeft => d.buffer.move_left(shift),
                Named::ArrowRight => d.buffer.move_right(shift),
                Named::ArrowUp => d.buffer.move_vertical(-1, shift),
                Named::ArrowDown => d.buffer.move_vertical(1, shift),
                Named::PageUp => d.buffer.move_vertical(-page, shift),
                Named::PageDown => d.buffer.move_vertical(page, shift),
                Named::Home if ctrl => d.buffer.doc_start(shift),
                Named::End if ctrl => d.buffer.doc_end(shift),
                Named::Home => d.buffer.home(shift),
                Named::End => d.buffer.end(shift),
                Named::Backspace => {
                    d.buffer.backspace();
                    d.edited(hl);
                }
                Named::Delete => {
                    d.buffer.delete();
                    d.edited(hl);
                }
                Named::Enter => {
                    d.buffer.insert_newline();
                    d.edited(hl);
                }
                Named::Tab => {
                    d.buffer.insert(&" ".repeat(TAB_WIDTH));
                    d.edited(hl);
                }
                Named::Space => {
                    d.buffer.insert(" ");
                    d.edited(hl);
                }
                _ => {}
            }
            return Task::none();
        }

        if let Some(t) = text {
            let t: String = t.chars().filter(|c| !c.is_control()).collect();
            if !t.is_empty() && !mods.control() && !mods.alt() || mods.control() && mods.alt() {
                d.buffer.insert(&t);
                d.edited(hl);
            }
        }
        Task::none()
    }

    // --- view ---

    fn view(&self) -> Element<'_, Message> {
        let mut body = row![].height(Length::Fill);
        if self.show_sidebar {
            body = body.push(self.sidebar()).push(divider_v());
        }
        body = body.push(self.center());
        if self.show_ai {
            body = body.push(divider_v()).push(self.ai_panel());
        }
        let base = column![self.top_bar(), divider_h(), body, divider_h(), self.status_bar()];
        match &self.palette {
            Some(p) => stack![
                base,
                mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill))
                    .on_press(Message::ClosePalette),
                self.palette_view(p),
            ]
            .into(),
            None => base.into(),
        }
    }

    fn palette_view(&self, p: &Palette) -> Element<'_, Message> {
        let results = palette::filter(&p.query);
        let list: Element<'_, Message> = if results.is_empty() {
            container(text("No matching commands").size(13).color(theme::TEXT_DIM))
                .padding(12)
                .into()
        } else {
            column(results.into_iter().enumerate().map(|(i, cmd)| {
                button(
                    row![
                        text(cmd.label).size(14),
                        space::horizontal(),
                        text(cmd.shortcut).size(12).color(theme::TEXT_DIM),
                    ]
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([7, 12])
                .style(theme::flat(i == p.selected))
                .on_press(Message::RunPalette(i))
                .into()
            }))
            .padding(4)
            .into()
        };
        let input = container(
            text(if p.query.is_empty() {
                "Type a command…".to_string()
            } else {
                format!("{}▏", p.query)
            })
            .size(15)
            .color(if p.query.is_empty() { theme::TEXT_DIM } else { theme::TEXT }),
        )
        .padding([10, 14])
        .width(Length::Fill);
        let panel = container(column![input, divider_h(), list])
            .width(Length::Fixed(520.0))
            .style(|_: &Theme| container::Style {
                background: Some(theme::PANEL_BG.into()),
                border: iced::Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5),
                    offset: iced::Vector::new(0.0, 8.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            });
        container(opaque(panel))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .padding(iced::Padding::ZERO.top(70.0))
            .into()
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let project = self.root.as_ref().map(|r| r.name.as_str()).unwrap_or("No folder open");
        container(
            row![
                button(text(format!("◆  {project}")).size(13))
                    .style(theme::flat(false))
                    .padding([3, 10])
                    .on_press(Message::OpenFolderDialog),
                space::horizontal(),
                text(self.docs.get(self.panes.cur()).map(|d| d.name()).unwrap_or_default())
                    .size(12)
                    .color(theme::TEXT_DIM),
                space::horizontal().width(Length::Fixed(80.0)),
            ]
            .align_y(Alignment::Center)
            .padding([4, 8]),
        )
        .width(Length::Fill)
        .style(theme::bg(theme::BAR_BG))
        .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.root {
            None => column![
                text("No folder open").size(13).color(theme::TEXT_DIM),
                button(text("Open Folder").size(13))
                    .style(theme::flat(false))
                    .padding([4, 10])
                    .on_press(Message::OpenFolderDialog),
            ]
            .spacing(10)
            .padding(16)
            .into(),
            Some(root) => {
                let mut rows = Vec::new();
                root.rows(0, &mut rows);
                let active_path = self.docs.get(self.panes.cur()).and_then(|d| d.path.as_ref());
                let list = column(rows.into_iter().map(|(depth, node)| {
                    let arrow = if !node.is_dir {
                        " "
                    } else if node.expanded {
                        "▾"
                    } else {
                        "▸"
                    };
                    let msg = if node.is_dir {
                        Message::ToggleDir(node.path.clone())
                    } else {
                        Message::OpenFile(node.path.clone())
                    };
                    let selected = active_path == Some(&node.path);
                    button(
                        row![
                            space::horizontal().width(Length::Fixed(depth as f32 * 12.0)),
                            text(arrow).size(11),
                            text(node.name.clone()).size(13),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([3, 8])
                    .style(theme::flat(selected))
                    .on_press(msg)
                    .into()
                }))
                .padding([0, 6]);
                column![
                    text(root.name.to_uppercase())
                        .size(11)
                        .color(theme::TEXT_DIM)
                        .width(Length::Fill),
                    scrollable(list).height(Length::Fill),
                ]
                .spacing(8)
                .padding([10, 6])
                .into()
            }
        };
        container(content)
            .width(Length::Fixed(240.0))
            .height(Length::Fill)
            .style(theme::bg(theme::PANEL_BG))
            .into()
    }

    fn center(&self) -> Element<'_, Message> {
        let single = self.panes.list.len() == 1;
        if single && self.panes.list[0].tabs.is_empty() {
            return container(self.welcome())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::bg(theme::EDITOR_BG))
                .into();
        }
        let mut views = Vec::new();
        for i in 0..self.panes.list.len() {
            if i > 0 {
                views.push(match self.panes.axis {
                    Axis::Horizontal => divider_v(),
                    Axis::Vertical => divider_h(),
                });
            }
            views.push(self.pane_view(i));
        }
        match self.panes.axis {
            Axis::Horizontal => row(views).width(Length::Fill).height(Length::Fill).into(),
            Axis::Vertical => column(views).width(Length::Fill).height(Length::Fill).into(),
        }
    }

    fn pane_view(&self, index: usize) -> Element<'_, Message> {
        let pane = &self.panes.list[index];
        let focused = index == self.panes.focus;
        let body: Element<'_, Message> = match pane.current().and_then(|t| self.docs.get(t.doc).map(|d| (t, d))) {
            Some((tab, doc)) => Canvas::new(EditorView {
                doc,
                scroll: tab.scroll,
                pane: index,
                focused,
                viewport: &pane.viewport,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            None => container(text("Open a file here").size(13).color(theme::TEXT_DIM))
                .center(Length::Fill)
                .into(),
        };
        container(column![self.tab_bar(index), divider_h(), body])
            .width(Length::FillPortion(1))
            .height(Length::FillPortion(1))
            .style(theme::bg(theme::EDITOR_BG))
            .into()
    }

    fn tab_bar(&self, pane_index: usize) -> Element<'_, Message> {
        let pane = &self.panes.list[pane_index];
        let focused = pane_index == self.panes.focus;
        let tabs = pane.tabs.iter().enumerate().filter_map(|(i, t)| {
            let d = self.docs.get(t.doc)?;
            let active = i == pane.active;
            let marker = if d.dirty { "●" } else { "×" };
            let tab = row![
                button(text(d.name()).size(13))
                    .padding([6, 4])
                    .style(theme::flat(false))
                    .on_press(Message::SelectTab { pane: pane_index, tab: i }),
                button(text(marker).size(12))
                    .padding([6, 6])
                    .style(theme::flat(false))
                    .on_press(Message::CloseTab { pane: pane_index, tab: i }),
            ]
            .align_y(Alignment::Center)
            .padding([0, 6]);
            let accent_color = match (active, focused) {
                (true, true) => theme::ACCENT,
                (true, false) => theme::TEXT_DIM,
                _ => theme::PANEL_BG,
            };
            let accent = container(space::vertical().height(Length::Fixed(2.0)))
                .width(Length::Fill)
                .style(theme::bg(accent_color));
            Some(
                container(column![accent, tab])
                    .style(theme::bg(if active { theme::EDITOR_BG } else { theme::PANEL_BG }))
                    .into(),
            )
        });
        container(scrollable(row(tabs)).direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(0).scroller_width(0),
        )))
        .width(Length::Fill)
        .style(theme::bg(theme::PANEL_BG))
        .into()
    }

    fn welcome(&self) -> Element<'_, Message> {
        let action = |label: &'static str, hint: &'static str, msg: Message| {
            button(
                row![text(label).size(14), space::horizontal(), text(hint).size(12).color(theme::TEXT_DIM)]
                    .align_y(Alignment::Center),
            )
            .width(Length::Fixed(340.0))
            .padding([8, 12])
            .style(theme::flat(false))
            .on_press(msg)
        };
        container(
            column![
                text("Hythe").size(34).color(theme::ACCENT),
                text("a fast editor, built in Rust").size(14).color(theme::TEXT_DIM),
                space::vertical().height(Length::Fixed(24.0)),
                action("New File", "Ctrl+N", Message::NewFile),
                action("Open File", "Ctrl+O", Message::OpenFileDialog),
                action("Open Folder", "Ctrl+Shift+O", Message::OpenFolderDialog),
                space::vertical().height(Length::Fixed(14.0)),
                text("Ctrl+B toggles the sidebar").size(12).color(theme::TEXT_DIM),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into()
    }

    fn ai_panel(&self) -> Element<'_, Message> {
        container(
            column![
                text("ASSISTANT").size(11).color(theme::TEXT_DIM),
                space::vertical().height(Length::Fill),
                text("Connect Claude or OpenAI to chat about your code.")
                    .size(13)
                    .color(theme::TEXT_DIM),
                text("Coming in Phase 3.").size(12).color(theme::TEXT_DIM),
                space::vertical().height(Length::Fill),
                container(text("Ask anything…").size(13).color(theme::TEXT_DIM))
                    .padding(10)
                    .width(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        background: Some(theme::EDITOR_BG.into()),
                        border: iced::Border {
                            color: theme::BORDER,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    }),
            ]
            .spacing(8)
            .padding(12),
        )
        .width(Length::Fixed(320.0))
        .height(Length::Fill)
        .style(theme::bg(theme::PANEL_BG))
        .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let toggle = |label: &'static str, on: bool, msg: Message| {
            button(text(label).size(12))
                .padding([2, 8])
                .style(theme::flat(on))
                .on_press(msg)
        };
        let position = self
            .docs
            .get(self.panes.cur())
            .map(|d| {
                let (l, c) = d.buffer.cursor();
                format!("Ln {}, Col {}    {}", l + 1, c + 1, d.language())
            })
            .unwrap_or_default();
        container(
            row![
                toggle("Explorer", self.show_sidebar, Message::ToggleSidebar),
                text(self.status.clone()).size(12).color(theme::TEXT_DIM),
                space::horizontal(),
                text(position).size(12).color(theme::TEXT_DIM),
                toggle("Split", self.panes.list.len() > 1, Message::Split(Axis::Horizontal)),
                toggle("Assistant", self.show_ai, Message::ToggleAi),
            ]
            .spacing(14)
            .align_y(Alignment::Center)
            .padding([3, 8]),
        )
        .width(Length::Fill)
        .style(theme::bg(theme::BAR_BG))
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().map(Message::Key),
            window::close_requests().map(|_| Message::CloseRequested),
        ])
    }
}

fn divider_v<'a>() -> Element<'a, Message> {
    container(space::horizontal().width(Length::Fixed(1.0)))
        .height(Length::Fill)
        .style(theme::bg(theme::BORDER))
        .into()
}

fn divider_h<'a>() -> Element<'a, Message> {
    container(space::vertical().height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .style(theme::bg(theme::BORDER))
        .into()
}
