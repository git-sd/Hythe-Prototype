mod buffer;
mod doc;
mod editor;
mod explorer;
mod highlight;
mod palette;
mod panes;
mod terminal_view;
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
use hythe_term::{Profile, SessionId, Snapshot, TerminalManager};
use palette::Palette;
use panes::{Axis, Panes};
use terminal_view::TerminalView;

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
    show_term: bool,
    /// Keyboard focus is in the terminal panel rather than the editor.
    focus_term: bool,
    terms: TerminalManager,
    term_active: Option<SessionId>,
    term_snap: Option<Snapshot>,
    /// CLI agents found on PATH, offered as terminal profiles.
    agents: Vec<Profile>,
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
    ToggleTerminal,
    /// New terminal running the default shell (`None`) or the n-th detected agent.
    NewTerminal(Option<usize>),
    CloseTerminal,
    CloseTermTab(SessionId),
    TermSelect(SessionId),
    TermPress { row: usize, col: usize },
    TermDrag { row: usize, col: usize },
    TermScroll(i32),
    TermPaste(Option<String>),
    TermTick,
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
            show_term: false,
            focus_term: false,
            terms: TerminalManager::default(),
            term_active: None,
            term_snap: None,
            agents: hythe_term::available_agents(),
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

    fn agent_names(&self) -> Vec<String> {
        self.agents.iter().map(|a| a.name.clone()).collect()
    }

    fn refresh_term_snapshot(&mut self) {
        self.term_snap = self.term_active.and_then(|id| self.terms.get(id)).map(|s| s.snapshot());
    }

    /// Start a shell (`None`) or the n-th detected agent in the project folder.
    fn new_terminal(&mut self, profile: Option<usize>) {
        let profile = match profile {
            Some(i) => match self.agents.get(i) {
                Some(p) => p.clone(),
                None => return,
            },
            None => Profile::shell(),
        };
        let cwd = self.root.as_ref().map(|r| r.path.clone());
        match self.terms.spawn_profile(&profile, cwd) {
            Ok(id) => {
                self.term_active = Some(id);
                self.show_term = true;
                self.focus_term = true;
                self.refresh_term_snapshot();
            }
            Err(e) => self.status = format!("Could not start {}: {e}", profile.name),
        }
    }

    fn close_terminal(&mut self, id: SessionId) {
        self.terms.close(id);
        if self.term_active == Some(id) {
            self.term_active = self.terms.sessions().last().map(|s| s.id());
            self.refresh_term_snapshot();
        }
        if self.terms.sessions().is_empty() {
            self.term_snap = None;
            self.focus_term = false;
        }
    }

    /// Keys while the terminal has focus: almost everything goes to the
    /// running program, so shell and agent shortcuts (Ctrl+C, Ctrl+W, …) work.
    fn term_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<&str>) -> Task<Message> {
        use hythe_term as ht;
        let shift = mods.shift();

        if let Key::Character(c) = key.as_ref() {
            let c = c.to_lowercase();
            if mods.command() && shift {
                match c.as_str() {
                    "p" => return self.update(Message::OpenPalette),
                    "c" => {
                        let copied = self
                            .term_active
                            .and_then(|id| self.terms.get(id))
                            .and_then(|s| s.selected_text());
                        if let Some(t) = copied {
                            return clipboard::write(t);
                        }
                        return Task::none();
                    }
                    "v" => return clipboard::read().map(Message::TermPaste),
                    "w" => return self.update(Message::CloseTerminal),
                    "`" | "~" => return self.update(Message::NewTerminal(None)),
                    _ => {}
                }
            }
            if mods.command() && !shift && c == "`" {
                return self.update(Message::ToggleTerminal);
            }
        }

        let Some(session) = self.term_active.and_then(|id| self.terms.get(id)) else {
            return Task::none();
        };
        if !session.is_running() {
            return Task::none();
        }
        let m = ht::Mods { ctrl: mods.control(), alt: mods.alt(), shift };

        match key.as_ref() {
            Key::Named(named) => {
                let k = match named {
                    Named::Enter => Some(ht::Key::Enter),
                    Named::Tab => Some(ht::Key::Tab),
                    Named::Backspace => Some(ht::Key::Backspace),
                    Named::Escape => Some(ht::Key::Escape),
                    Named::Space => Some(ht::Key::Char(' ')),
                    Named::ArrowUp => Some(ht::Key::Up),
                    Named::ArrowDown => Some(ht::Key::Down),
                    Named::ArrowLeft => Some(ht::Key::Left),
                    Named::ArrowRight => Some(ht::Key::Right),
                    Named::Home => Some(ht::Key::Home),
                    Named::End => Some(ht::Key::End),
                    Named::PageUp => Some(ht::Key::PageUp),
                    Named::PageDown => Some(ht::Key::PageDown),
                    Named::Insert => Some(ht::Key::Insert),
                    Named::Delete => Some(ht::Key::Delete),
                    Named::F1 => Some(ht::Key::F(1)),
                    Named::F2 => Some(ht::Key::F(2)),
                    Named::F3 => Some(ht::Key::F(3)),
                    Named::F4 => Some(ht::Key::F(4)),
                    Named::F5 => Some(ht::Key::F(5)),
                    Named::F6 => Some(ht::Key::F(6)),
                    Named::F7 => Some(ht::Key::F(7)),
                    Named::F8 => Some(ht::Key::F(8)),
                    Named::F9 => Some(ht::Key::F(9)),
                    Named::F10 => Some(ht::Key::F(10)),
                    Named::F11 => Some(ht::Key::F(11)),
                    Named::F12 => Some(ht::Key::F(12)),
                    _ => None,
                };
                if let Some(k) = k {
                    session.send_key(&k, m);
                }
            }
            Key::Character(c) => {
                // Ctrl+Alt together is AltGr on many layouts, so treat it as text.
                if (m.ctrl || m.alt) && !(m.ctrl && m.alt) {
                    if let Some(ch) = c.chars().next() {
                        session.send_key(&ht::Key::Char(ch), m);
                    }
                } else if let Some(t) = text {
                    let t: String = t.chars().filter(|c| !c.is_control()).collect();
                    if !t.is_empty() {
                        session.scroll_to_bottom();
                        session.write(t.into_bytes());
                    }
                }
            }
            _ => {}
        }
        Task::none()
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
                self.focus_term = false;
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
                if self.show_term && self.focus_term {
                    return self.term_key(key, modifiers, text.as_deref());
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
            Message::ToggleTerminal => {
                if !self.show_term {
                    self.show_term = true;
                    self.focus_term = true;
                    if self.terms.sessions().is_empty() {
                        self.new_terminal(None);
                    } else {
                        self.refresh_term_snapshot();
                    }
                } else if self.focus_term {
                    self.show_term = false;
                    self.focus_term = false;
                } else {
                    self.focus_term = true;
                }
            }
            Message::NewTerminal(profile) => self.new_terminal(profile),
            Message::CloseTerminal => {
                if let Some(id) = self.term_active {
                    self.close_terminal(id);
                }
            }
            Message::CloseTermTab(id) => self.close_terminal(id),
            Message::TermSelect(id) => {
                self.term_active = Some(id);
                self.focus_term = true;
                self.refresh_term_snapshot();
            }
            Message::TermPress { row, col } => {
                self.focus_term = true;
                if let Some(s) = self.term_active.and_then(|id| self.terms.get(id)) {
                    s.clear_selection();
                    s.start_selection(row, col);
                }
            }
            Message::TermDrag { row, col } => {
                if let Some(s) = self.term_active.and_then(|id| self.terms.get(id)) {
                    s.update_selection(row, col);
                }
            }
            Message::TermScroll(lines) => {
                if let Some(s) = self.term_active.and_then(|id| self.terms.get(id)) {
                    if self.term_snap.as_ref().is_some_and(|n| n.alt_screen) {
                        // Full-screen apps have no scrollback; wheel moves their cursor.
                        let key = if lines > 0 { hythe_term::Key::Up } else { hythe_term::Key::Down };
                        for _ in 0..lines.unsigned_abs() {
                            s.send_key(&key, hythe_term::Mods::default());
                        }
                    } else {
                        s.scroll(lines);
                    }
                }
            }
            Message::TermPaste(Some(text)) => {
                if let Some(s) = self.term_active.and_then(|id| self.terms.get(id)) {
                    s.paste(&text);
                }
            }
            Message::TermPaste(None) => {}
            Message::TermTick => {
                let dirty = self.terms.take_dirty();
                if self.term_active.is_some_and(|a| dirty.contains(&a)) {
                    self.refresh_term_snapshot();
                }
            }
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
                if let Some(cmd) = palette::filter(&query, &self.agent_names()).into_iter().nth(i) {
                    return self.update(cmd.message);
                }
            }
        }
        Task::none()
    }

    fn palette_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<&str>) -> Task<Message> {
        let names = self.agent_names();
        let Some(p) = self.palette.as_mut() else {
            return Task::none();
        };
        let count = palette::filter(&p.query, &names).len();
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
                    ("`", false) => return self.update(Message::ToggleTerminal),
                    ("`", true) | ("~", _) => return self.update(Message::NewTerminal(None)),
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
        if self.show_term {
            body = body.push(divider_v()).push(self.terminal_panel());
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
        let results = palette::filter(&p.query, &self.agent_names());
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

    fn terminal_panel(&self) -> Element<'_, Message> {
        let small = |label: String, selected: bool, msg: Message| {
            button(text(label).size(12))
                .padding([4, 8])
                .style(theme::flat(selected))
                .on_press(msg)
        };

        let mut header = row![].spacing(2).align_y(Alignment::Center).padding([2, 6]);
        for info in self.terms.list() {
            let active = self.term_active == Some(info.id);
            let label = if info.running { info.name.clone() } else { format!("{} (exited)", info.name) };
            header = header
                .push(small(label, active, Message::TermSelect(info.id)))
                .push(small("×".into(), false, Message::CloseTermTab(info.id)));
        }
        header = header.push(space::horizontal());
        header = header.push(small("+ Shell".into(), false, Message::NewTerminal(None)));
        for (i, agent) in self.agents.iter().enumerate() {
            header = header.push(small(format!("▸ {}", agent.name), false, Message::NewTerminal(Some(i))));
        }

        let session = self.term_active.and_then(|id| self.terms.get(id));
        let body: Element<'_, Message> = match (&self.term_snap, session) {
            (Some(snap), Some(session)) => Canvas::new(TerminalView {
                snap,
                session,
                focused: self.focus_term,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            _ => container(
                column![
                    text("No terminal running").size(14),
                    button(text("New Terminal").size(13))
                        .padding([6, 12])
                        .style(theme::flat(false))
                        .on_press(Message::NewTerminal(None)),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into(),
        };

        let focus_line = container(space::vertical().height(Length::Fixed(2.0)))
            .width(Length::Fill)
            .style(theme::bg(if self.focus_term { theme::ACCENT } else { theme::PANEL_BG }));
        container(column![focus_line, header, divider_h(), body])
            .width(Length::Fixed(560.0))
            .height(Length::Fill)
            .style(theme::bg(theme::EDITOR_BG))
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
                toggle("Terminal", self.show_term, Message::ToggleTerminal),
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
        let mut subs = vec![
            keyboard::listen().map(Message::Key),
            window::close_requests().map(|_| Message::CloseRequested),
        ];
        // Poll terminal output at ~60Hz, but only while it's on screen.
        if self.show_term && !self.terms.sessions().is_empty() {
            subs.push(iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::TermTick));
        }
        Subscription::batch(subs)
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
