mod buffer;
mod completion_ui;
mod doc;
mod editor;
mod explorer;
mod highlight;
mod icons;
mod menus;
mod palette;
mod panes;
mod state;
mod terminal_view;
mod theme;
mod view;

use std::path::PathBuf;

use iced::keyboard::{self, key::Named, Key};
use iced::{clipboard, window, Subscription, Task, Theme};

use doc::{Doc, TAB_WIDTH};
use explorer::Node;
use completion_ui::CompletionUi;
use highlight::Highlighter;
use hythe_term::{Profile, SessionId, Snapshot, TerminalManager};
use palette::Palette;
use panes::{Axis, Panes};
use state::{GitInfo, Persisted};

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Dark)
        .window_size((1360.0, 820.0))
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
    /// Open top-bar dropdown (index into `menus::MENUS`).
    menu: Option<usize>,
    /// Open completion popup in the focused editor.
    completion: Option<CompletionUi>,
    highlighter: Highlighter,
    /// Recent projects and pinned files, kept between runs.
    persisted: Persisted,
    /// Branch and change counts for the open project.
    git: Option<GitInfo>,
    /// Name shown on the account chip.
    user: String,
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
    /// Pick entry `n` of the open completion popup (mouse click).
    CompletionPick(usize),
    /// Ctrl+Space: show suggestions now.
    TriggerCompletion,
    ToggleSidebar,
    OpenMenu(usize),
    CloseMenu,
    /// A menu entry was clicked: close the menu, then run the message.
    MenuPick(Box<Message>),
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
    /// Open a project from the sidebar's Projects list.
    OpenRecent(PathBuf),
    /// Pin or unpin the current file in the sidebar.
    TogglePin,
    /// Re-read branch and change counts from git.
    GitRefresh,
    /// A button for a feature that isn't built yet.
    Unavailable(&'static str),
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
            show_term: true,
            focus_term: false,
            terms: TerminalManager::default(),
            term_active: None,
            term_snap: None,
            agents: hythe_term::available_agents(),
            status: String::new(),
            palette: None,
            menu: None,
            completion: None,
            highlighter: Highlighter::new(),
            persisted: Persisted::load(),
            git: None,
            user: state::git_user().unwrap_or_else(|| "you".into()),
        };
        if let Some(arg) = std::env::args().nth(1) {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                app.open_folder(path);
            } else {
                app.open_file(path);
            }
        }
        app
    }

    /// Make `path` the project: show it in the sidebar and remember it.
    fn open_folder(&mut self, path: PathBuf) {
        self.persisted.remember_project(&path);
        self.persisted.save();
        self.root = Some(Node::root(path));
        self.show_sidebar = true;
        self.refresh_git();
    }

    fn refresh_git(&mut self) {
        self.git = self.root.as_ref().and_then(|r| state::git_info(&r.path));
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
                    // Show the file's folder without adding it to recent projects.
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        self.root = Some(Node::root(parent.to_path_buf()));
                        self.refresh_git();
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

    /// Recompute suggestions at the cursor. `explicit` is a request (Ctrl+Space)
    /// rather than a side effect of typing.
    fn refresh_completion(&mut self, explicit: bool) {
        self.completion = None;
        let idx = self.panes.cur();
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        if doc.buffer.has_selection() {
            return;
        }
        let text = doc.buffer.text();
        let char_cursor = doc.buffer.cursor_index();
        let cursor = doc.buffer.char_to_byte(char_cursor);

        // Symbols from other open files, bounded so typing stays instant.
        let others: Vec<String> = self
            .docs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .take(8)
            .map(|(_, d)| d.buffer.text())
            .filter(|t| t.len() < 400_000)
            .collect();
        let other_refs: Vec<&str> = others.iter().map(String::as_str).collect();

        let result = hythe_complete::complete(&hythe_complete::Request {
            language: doc.lang(),
            text: &text,
            cursor,
            explicit,
            others: &other_refs,
        });
        if result.items.is_empty() {
            return;
        }
        // Already fully typed and nothing else to offer: stay out of the way.
        if result.items.len() == 1 && result.items[0].label == text[result.start..cursor] {
            return;
        }
        self.completion = Some(CompletionUi {
            items: result.items,
            selected: 0,
            navigated: false,
            start_char: doc.buffer.byte_to_char(result.start),
        });
    }

    fn accept_completion(&mut self) {
        let Some(ui) = self.completion.take() else {
            return;
        };
        let Some(item) = ui.items.get(ui.selected) else {
            return;
        };
        let Some(d) = self.docs.get_mut(self.panes.cur()) else {
            return;
        };
        let (line, _) = d.buffer.cursor();
        let indent: String = d
            .buffer
            .line_text(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let expanded = item.expand(&indent);
        d.buffer.apply_completion(ui.start_char, &expanded.text, expanded.select);
        d.edited(&mut self.highlighter);
        self.reveal_cursor();
    }

    /// Keys that drive the popup. Returns true if the key was consumed.
    fn completion_key(&mut self, key: &Key, mods: keyboard::Modifiers) -> bool {
        if mods.command() && matches!(key, Key::Named(Named::Space)) {
            self.refresh_completion(true);
            return true;
        }
        let Some(ui) = self.completion.as_mut() else {
            return false;
        };
        match key {
            Key::Named(Named::ArrowDown) => {
                ui.selected = (ui.selected + 1).min(ui.items.len() - 1);
                ui.navigated = true;
                true
            }
            Key::Named(Named::ArrowUp) => {
                ui.selected = ui.selected.saturating_sub(1);
                ui.navigated = true;
                true
            }
            Key::Named(Named::Tab) if !mods.shift() => {
                self.accept_completion();
                true
            }
            // Enter only accepts once the user has chosen an entry, so
            // typing a word and pressing Enter still starts a new line.
            Key::Named(Named::Enter) => {
                if ui.navigated {
                    self.accept_completion();
                    true
                } else {
                    self.completion = None;
                    false
                }
            }
            Key::Named(Named::Escape) => {
                self.completion = None;
                true
            }
            Key::Named(
                Named::ArrowLeft | Named::ArrowRight | Named::Home | Named::End | Named::PageUp | Named::PageDown,
            ) => {
                self.completion = None;
                false
            }
            _ => false,
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
        // The completion popup belongs to one cursor position; almost anything
        // other than typing, scrolling or picking an entry invalidates it.
        if !matches!(
            message,
            Message::Key(_)
                | Message::TermTick
                | Message::Scroll { .. }
                | Message::TermScroll(_)
                | Message::TermDrag { .. }
                | Message::CompletionPick(_)
        ) {
            self.completion = None;
        }
        match message {
            Message::CompletionPick(i) => {
                if let Some(ui) = self.completion.as_mut() {
                    ui.selected = i.min(ui.items.len().saturating_sub(1));
                }
                self.accept_completion();
            }
            Message::TriggerCompletion => self.refresh_completion(true),
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
                // Any key press dismisses an open dropdown; Esc is just the dismissal.
                if self.menu.take().is_some() && matches!(key, Key::Named(Named::Escape)) {
                    return Task::none();
                }
                if self.palette.is_some() {
                    return self.palette_key(key, modifiers, text.as_deref());
                }
                if self.show_term && self.focus_term {
                    return self.term_key(key, modifiers, text.as_deref());
                }
                if self.completion_key(&key, modifiers) {
                    self.reveal_cursor();
                    return Task::none();
                }
                // Typing (or backspacing) a word is what brings suggestions up.
                let typed = match &key {
                    Key::Character(_) => !modifiers.command(),
                    Key::Named(Named::Backspace | Named::Space) => !modifiers.command(),
                    _ => false,
                };
                let task = self.handle_key(key, modifiers, text.as_deref());
                self.reveal_cursor();
                if typed {
                    self.refresh_completion(false);
                }
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
                    self.open_folder(p);
                }
            }
            Message::OpenRecent(p) => {
                if p.is_dir() {
                    self.open_folder(p);
                } else {
                    self.status = format!("{} no longer exists", p.display());
                    self.persisted.recents.retain(|r| r != &p);
                    self.persisted.save();
                }
            }
            Message::TogglePin => {
                let current = self
                    .docs
                    .get(self.panes.cur())
                    .and_then(|d| d.path.clone());
                match current {
                    Some(path) => {
                        let pinned = self.persisted.toggle_pin(&path);
                        self.persisted.save();
                        self.status = format!("{} {}", if pinned { "Pinned" } else { "Unpinned" }, path.display());
                    }
                    None => self.status = "Save the file before pinning it".into(),
                }
            }
            Message::GitRefresh => self.refresh_git(),
            Message::Unavailable(what) => self.status = format!("{what} isn't available yet"),
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
            Message::OpenMenu(i) => {
                // Clicking the open menu's button again closes it.
                self.menu = if self.menu == Some(i) { None } else { Some(i) };
            }
            Message::CloseMenu => self.menu = None,
            Message::MenuPick(inner) => {
                self.menu = None;
                return self.update(*inner);
            }
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
                    self.refresh_git();
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

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            keyboard::listen().map(Message::Key),
            window::close_requests().map(|_| Message::CloseRequested),
        ];
        // Branch name and change counts drift as files are edited on disk.
        if self.root.is_some() {
            subs.push(iced::time::every(std::time::Duration::from_secs(6)).map(|_| Message::GitRefresh));
        }
        // Poll terminal output at ~60Hz, but only while it's on screen.
        if self.show_term && !self.terms.sessions().is_empty() {
            subs.push(iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::TermTick));
        }
        Subscription::batch(subs)
    }
}
