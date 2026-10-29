mod add_repo;
mod avatar;
mod buffer;
mod chat;
mod collab;
mod completion_ui;
mod doc;
mod editor;
mod explorer;
mod github;
mod highlight;
mod icons;
mod menus;
mod palette;
mod panes;
mod scm;
mod scm_view;
mod state;
mod terminal_view;
mod theme;
mod view;
mod workspace;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use iced::keyboard::{self, key::Named, Key};
use iced::{clipboard, window, Subscription, Task, Theme};

use doc::{Doc, TAB_WIDTH};
use explorer::Node;
use add_repo::{AddRepo, AddRepoMsg};
use chat::{Chat, ChatMsg};
use collab::{Collab, CollabMsg, CollabUi};
use completion_ui::CompletionUi;
use highlight::Highlighter;
use hythe_term::{Profile, SessionId, Snapshot, TerminalManager};
use palette::Palette;
use panes::{Axis, Panes};
use scm::{Scm, ScmMsg};
use state::{GitInfo, Persisted};
use workspace::{Project, Workspace};

/// People who committed, one entry per person, most active first.
fn members_of(repo: &hythe_git::Repo) -> Vec<Member> {
    let mut seen = HashSet::new();
    repo.contributors()
        .into_iter()
        .filter_map(|c| {
            let login = hythe_git::noreply_login(&c.email);
            let key = login.clone().unwrap_or_else(|| c.name.to_lowercase());
            seen.insert(key).then(|| Member { name: c.name, login, commits: c.commits })
        })
        .take(40)
        .collect()
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Dark)
        // Start modest and centred; `FitWindow` then shrinks it to the monitor if needed.
        .window(window::Settings {
            size: iced::Size::new(1280.0, 780.0),
            min_size: Some(iced::Size::new(860.0, 520.0)),
            position: window::Position::Centered,
            resizable: true,
            decorations: false,
            ..window::Settings::default()
        })
        .exit_on_close_request(false)
        .run()
}

pub(crate) enum Unsaved {
    Save,
    Discard,
    Cancel,
}

pub(crate) fn ask_unsaved(body: &str, save_label: &str) -> Unsaved {
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
    repo: Option<hythe_git::Repo>,
    scm: Scm,
    /// Show who last changed the cursor line.
    blame_on: bool,
    /// Name shown on the account chip.
    user: String,
    /// Repositories in the rail; the open one's state lives in the fields above.
    projects: Vec<Project>,
    active_project: Option<usize>,
    /// Workspace shown when no repository is selected (the Home screen).
    home: Option<Workspace>,
    /// Live collaboration for the open repository.
    collab: Option<Collab>,
    collab_ui: CollabUi,
    add_repo: AddRepo,
    /// The agent dropdown in the empty thread panel is open.
    agent_menu: bool,
    /// Open submenu in the cascading main menu (index into `menus::MENUS`).
    submenu: usize,
    /// GitHub profile picture for the account chip.
    avatar: Option<iced::widget::image::Handle>,
    avatar_tried: bool,
    /// Column widths, changed by dragging the dividers.
    list_w: f32,
    agent_w: f32,
    scm_w: f32,
    /// Window size, for keeping dragged columns on screen.
    win: iced::Size,
    drag: Option<Drag>,
    /// Everyone who has committed to the open repository.
    members: Vec<Member>,
    members_for: Option<PathBuf>,
    member_avatars: HashMap<String, iced::widget::image::Handle>,
    member_avatar_tried: HashSet<String>,
    /// Git is read off the UI thread; these track the request in flight.
    git_wanted: bool,
    git_busy: bool,
    chat: Chat,
    /// A tab being dragged: (pane, tab).
    tab_drag: Option<(usize, usize)>,
    /// A rail tile, terminal or pinned file being dragged.
    item_drag: Option<Item>,
    /// The thread column is docked on the right instead of the left.
    agent_right: bool,
    chat_w: f32,
    /// The chat panel is docked on the right (otherwise the left).
    chat_right: bool,
}

/// Something in a list that can be dragged to a new place in it.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Rail(usize),
    Term(SessionId),
    Pin(PathBuf),
}

/// What a background git read found.
#[derive(Debug, Clone)]
pub struct GitResult {
    for_root: Option<PathBuf>,
    repo: Option<hythe_git::Repo>,
    info: Option<GitInfo>,
    /// `None`: the member list is already current.
    members: Option<Vec<Member>>,
}

/// A divider that can be dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Handle {
    List,
    Agent,
    Scm,
    /// The thread column's header: drag it to the other side of the window.
    Dock,
    Chat,
    /// The chat header: drag it to the other side of the window.
    ChatDock,
    /// Between editor pane `n` and `n + 1`.
    Panes(usize),
}

/// A drag in progress. For pane splits, `base` is (weight of pane n, weight of
/// pane n + 1, their combined size in pixels) when the drag began.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub handle: Handle,
    origin: Option<f32>,
    base: (f32, f32, f32),
}

/// A committer shown in the people panel.
#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    /// GitHub login, when the commit email reveals it.
    pub login: Option<String>,
    pub commits: u32,
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
    /// Source control: changes, commits, branches, blame.
    Scm(ScmMsg),
    /// Live collaboration through a relay.
    Collab(CollabMsg),
    /// The "Add a repository" dialog: local folder or clone.
    AddRepo(AddRepoMsg),
    /// Open or close the agent dropdown in the empty thread panel.
    AgentMenu(bool),
    /// Open a project from the sidebar's Projects list.
    OpenRecent(PathBuf),
    /// Click on a repository in the rail (None is the Home button).
    SwitchProject(Option<usize>),
    /// Remove the open repository from the rail.
    CloseProject,
    /// Pin or unpin the current file in the sidebar.
    TogglePin,
    /// Re-read branch and change counts from git.
    GitRefresh,
    /// A button for a feature that isn't built yet.
    Unavailable(&'static str),
    /// Hover or click on a top-level entry of the cascading menu.
    Submenu(usize),
    /// Shrink and centre the window to fit the monitor (None: size unknown).
    FitWindow(window::Id, Option<iced::Size>),
    ResetWindow,
    DragWindow,
    MinimizeWindow,
    ToggleMaximize,
    AvatarReady(Option<PathBuf>),
    MemberAvatar(String, Option<PathBuf>),
    DragStart(Handle),
    PointerMoved(iced::Point),
    PointerUp,
    WindowSized(iced::Size),
    GitLoaded(Option<Box<GitResult>>),
    /// Look for open files changed on disk by something else.
    DiskTick,
    /// Mouse pressed on a file tab: select it and start dragging it.
    TabPress { pane: usize, tab: usize },
    /// Mouse pressed on a rail tile, terminal or pinned file: select it and start dragging.
    ItemPress(Item),
    ItemEnter(Item),
    /// The pointer moved onto a tab; while dragging, the dragged tab takes its place.
    TabEnter { pane: usize, tab: usize },
    /// Repository chat.
    Chat(ChatMsg),
    /// Zoom the current tab's text: 1 in, -1 out, 0 back to normal.
    Zoom(i32),
    /// Look for files added or removed on disk.
    TreeTick,
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
    fn new() -> (Self, Task<Message>) {
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
            agents: hythe_term::known_agents(),
            status: String::new(),
            palette: None,
            menu: None,
            completion: None,
            highlighter: Highlighter::new(),
            persisted: Persisted::load(),
            git: None,
            repo: None,
            scm: Scm::default(),
            blame_on: false,
            user: state::git_user().unwrap_or_else(|| "you".into()),
            projects: Vec::new(),
            active_project: None,
            home: None,
            collab: None,
            collab_ui: CollabUi::default(),
            add_repo: AddRepo::default(),
            agent_menu: false,
            submenu: 0,
            avatar: None,
            avatar_tried: false,
            list_w: view::LIST_W,
            agent_w: view::AGENT_W,
            scm_w: view::SCM_W,
            win: iced::Size::new(1280.0, 780.0),
            drag: None,
            members: Vec::new(),
            members_for: None,
            member_avatars: HashMap::new(),
            member_avatar_tried: HashSet::new(),
            git_wanted: false,
            git_busy: false,
            // Closed until asked for; the title bar and bottom bar icons open it.
            chat: Chat { dismissed: true, ..Chat::default() },
            tab_drag: None,
            item_drag: None,
            agent_right: false,
            chat_w: view::CHAT_W,
            chat_right: true,
        };
        app.projects = app.persisted.projects.iter().map(|p| Project { path: p.clone(), ws: None }).collect();
        // Scripted runs: HYTHE_COLLAB="ws://host:port|room|name" joins a room on startup.
        let auto_join = std::env::var("HYTHE_COLLAB").ok();
        if let Some(arg) = std::env::args().nth(1) {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                app.open_folder(path);
            } else {
                app.open_file(path);
            }
        } else if let Some(i) = app.persisted.active.as_ref().and_then(|a| app.projects.iter().position(|p| &p.path == a)) {
            app.switch_project(Some(i));
        } else if !app.projects.is_empty() {
            // No Home screen: always land in a repository.
            app.switch_project(Some(0));
        }
        if let Some(spec) = auto_join {
            let mut parts = spec.splitn(3, '|');
            if let (Some(url), Some(room), Some(name)) = (parts.next(), parts.next(), parts.next()) {
                app.collab_ui.url = url.to_string();
                app.collab_ui.room = room.to_string();
                app.collab_ui.name = name.to_string();
                let _ = app.collab_update(CollabMsg::Connect);
            }
        }
        let task = Task::batch([app.fit_window_task(), app.avatar_task()]);
        (app, task)
    }

    fn fit_window_task(&self) -> Task<Message> {
        window::oldest().and_then(|id| window::monitor_size(id).map(move |s| Message::FitWindow(id, s)))
    }

    /// Fetch the GitHub profile picture once a repository (and so a username) is known.
    fn avatar_task(&mut self) -> Task<Message> {
        if self.avatar.is_some() || self.avatar_tried {
            return Task::none();
        }
        let user = self.repo.as_ref().and_then(|r| r.github_user()).or_else(|| {
            Some(self.user.clone()).filter(|u| hythe_git::is_github_login(u))
        });
        let Some(user) = user else { return Task::none() };
        self.avatar_tried = true;
        Task::perform(
            async move { tokio::task::spawn_blocking(move || avatar::fetch(&user)).await.ok().flatten() },
            Message::AvatarReady,
        )
    }

    /// GitHub login used to list repositories and to fetch the account picture.
    pub(crate) fn github_login(&self) -> Option<String> {
        self.repo
            .as_ref()
            .and_then(|r| r.github_user())
            .or_else(|| Some(self.user.clone()).filter(|u| hythe_git::is_github_login(u)))
    }

    /// Fetch pictures of committers and chat authors whose GitHub login is known.
    fn picture_tasks(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let logins: Vec<String> =
            self.members.iter().filter_map(|m| m.login.clone()).chain(self.chat_logins().cloned()).collect();
        for login in logins {
            if self.member_avatar_tried.insert(login.clone()) {
                let key = login.clone();
                tasks.push(Task::perform(
                    async move { tokio::task::spawn_blocking(move || avatar::fetch(&login)).await.ok().flatten() },
                    move |f| Message::MemberAvatar(key.clone(), f),
                ));
            }
        }
        Task::batch(tasks)
    }

    /// People panel: shown only when more than one person works on the repository.
    pub(crate) fn show_members(&self) -> bool {
        self.members.len() >= 2 || self.collab.as_ref().is_some_and(|c| !c.peers.is_empty())
    }

    /// Ask for branch, change counts and committers to be re-read. Each `git`
    /// call is a process launch, so the read happens off the UI thread (see
    /// [`App::git_task`]) and switching repositories never waits on it.
    fn refresh_git(&mut self) {
        self.git_wanted = true;
    }

    fn git_task(&mut self) -> Task<Message> {
        if !self.git_wanted || self.git_busy {
            return Task::none();
        }
        self.git_wanted = false;
        self.git_busy = true;
        let root = self.root.as_ref().map(|r| r.path.clone());
        // Only look for the repository again when the project changed.
        let known = self.repo.clone().filter(|r| root.as_ref().is_some_and(|p| p.starts_with(r.root())));
        let have_members = known.as_ref().is_some_and(|r| self.members_for.as_deref() == Some(r.root()));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let repo = known.or_else(|| root.as_ref().and_then(|r| hythe_git::Repo::open(r)));
                    let info = repo.as_ref().map(|r| {
                        let (added, removed) = r.summary();
                        GitInfo { repo: r.name(), branch: r.current_branch(), added, removed }
                    });
                    let members = match (&repo, have_members) {
                        (Some(r), false) => Some(members_of(r)),
                        (None, _) => Some(Vec::new()),
                        _ => None,
                    };
                    Box::new(GitResult { for_root: root, repo, info, members })
                })
                .await
                .ok()
            },
            Message::GitLoaded,
        )
    }

    fn title(&self) -> String {
        match self.docs.get(self.panes.cur()) {
            Some(d) => format!("{}{} — Hythe", d.name(), if d.dirty { " •" } else { "" }),
            None => "Hythe".into(),
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        // Opening a file always brings you back to the code.
        self.scm.open = false;
        if let Some(i) = self.docs.iter().position(|d| d.path.as_ref() == Some(&path)) {
            self.panes.open_doc(i);
            return;
        }
        match Doc::open(path.clone(), &mut self.highlighter) {
            Ok(doc) => {
                if self.root.is_none() {
                    // A file outside any repository in the rail: its folder becomes one.
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        self.open_folder(parent.to_path_buf());
                    }
                }
                self.docs.push(doc);
                self.panes.open_doc(self.docs.len() - 1);
                self.load_blame(self.docs.len() - 1);
                self.collab_open_doc(self.docs.len() - 1);
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
        self.agent_menu = false;
        let profile = match profile {
            Some(i) => match self.agents.get(i) {
                Some(p) => p.clone(),
                None => return,
            },
            None => Profile::shell(),
        };
        if profile.kind == hythe_term::ProfileKind::Agent && !profile.is_available() {
            self.status = format!("{} isn't installed (not found on PATH)", profile.name);
            return;
        }
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
                    "g" => return self.update(Message::Scm(ScmMsg::Show)),
                    "0" | ")" => return self.update(Message::ResetWindow),
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
        let task = self.update_inner(message);
        // Anything that edited the text or moved the cursor is sent to collaborators.
        self.collab_flush();
        Task::batch([task, self.git_task(), self.picture_tasks()])
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        // The completion popup belongs to one cursor position; almost anything
        // other than typing, scrolling or picking an entry invalidates it.
        if !matches!(
            message,
            Message::Key(_)
                | Message::TermTick
                | Message::Collab(CollabMsg::Tick)
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
                self.scm.open = false;
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
            Message::OpenRecent(p) => self.open_folder(p),
            Message::SwitchProject(i) => {
                self.switch_project(i);
                return self.avatar_task();
            }
            Message::CloseProject => {
                if let Some(i) = self.active_project {
                    self.close_project(i);
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
            Message::GitRefresh => {
                self.refresh_git();
                return self.avatar_task();
            }
            Message::Scm(m) => return self.scm_update(m),
            Message::Collab(m) => return self.collab_update(m),
            Message::AddRepo(m) => return self.add_repo_update(m),
            Message::AgentMenu(open) => self.agent_menu = open,
            Message::Submenu(i) => self.submenu = i,
            Message::FitWindow(id, size) => {
                let (w, h) = match size {
                    Some(m) => ((m.width * 0.9).min(1280.0), (m.height * 0.85).min(780.0)),
                    None => (1100.0, 700.0),
                };
                let (mw, mh) = size.map(|m| (m.width, m.height)).unwrap_or((w, h));
                return Task::batch([
                    window::maximize(id, false),
                    window::resize(id, iced::Size::new(w, h)),
                    window::move_to(id, iced::Point::new(((mw - w) / 2.0).max(0.0), ((mh - h) / 2.0).max(0.0))),
                ]);
            }
            Message::DragWindow => return window::oldest().and_then(window::drag),
            Message::MinimizeWindow => return window::oldest().and_then(|id| window::minimize(id, true)),
            Message::ResetWindow => return self.fit_window_task(),
            Message::ToggleMaximize => return window::oldest().and_then(window::toggle_maximize),
            Message::MemberAvatar(login, file) => {
                if let Some(h) = file.and_then(|f| std::fs::read(f).ok()).map(iced::widget::image::Handle::from_bytes) {
                    self.member_avatars.insert(login, h);
                }
            }
            Message::WindowSized(size) => self.win = size,
            Message::Chat(m) => return self.chat_update(m),
            Message::Zoom(step) => {
                let vp = self.panes.list[self.panes.focus].viewport.get();
                let focus = self.panes.focus;
                let active = self.panes.list[focus].active;
                let cur = self.panes.cur();
                if let (Some(tab), Some(d)) = (self.panes.list[focus].tabs.get_mut(active), self.docs.get(cur)) {
                    let percent = tab.scroll.zoom_by(step);
                    tab.scroll = tab.scroll.clamped(&d.buffer, vp);
                    self.status = format!("Zoom {percent}%");
                }
            }
            Message::DiskTick => self.reload_changed_files(),
            Message::TreeTick => {
                if let Some(root) = &mut self.root {
                    root.refresh();
                }
            }
            Message::GitLoaded(result) => {
                self.git_busy = false;
                if let Some(r) = result {
                    if r.for_root == self.root.as_ref().map(|n| n.path.clone()) {
                        let r = *r;
                        self.repo = r.repo;
                        self.git = r.info;
                        if let Some(m) = r.members {
                            self.members = m;
                            self.members_for = self.repo.as_ref().map(|x| x.root().to_path_buf());
                        }
                        self.scm_reload();
                        let here = self.repo.as_ref().map(|x| x.root().to_path_buf());
                        if here.is_some() && self.chat.loaded_for != here {
                            self.chat.loaded_for = here;
                            return Task::batch([self.chat_load(), self.chat_sync()]);
                        }
                    } else {
                        // Another repository was opened meanwhile.
                        self.git_wanted = true;
                    }
                }
            }
            Message::DragStart(handle) => self.start_drag(handle),
            Message::PointerUp => {
                self.drag = None;
                self.tab_drag = None;
                self.item_drag = None;
            }
            Message::ItemPress(item) => {
                let task = match &item {
                    Item::Rail(i) => self.update_inner(Message::SwitchProject(Some(*i))),
                    Item::Term(id) => self.update_inner(Message::TermSelect(*id)),
                    Item::Pin(p) => self.update_inner(Message::OpenFile(p.clone())),
                };
                self.item_drag = Some(item);
                return task;
            }
            Message::ItemEnter(target) => self.item_enter(target),
            Message::TabPress { pane, tab } => {
                let task = self.update_inner(Message::SelectTab { pane, tab });
                self.tab_drag = Some((pane, tab));
                return task;
            }
            Message::TabEnter { pane, tab } => {
                if let Some(from) = self.tab_drag {
                    self.tab_drag = Some(self.panes.move_tab(from, (pane, tab)));
                }
            }
            Message::PointerMoved(p) => self.drag_to(p),
            Message::AvatarReady(path) => self.avatar = path.and_then(|p| std::fs::read(p).ok()).map(iced::widget::image::Handle::from_bytes),
            Message::Unavailable(what) => self.status = format!("{what} isn't available yet"),
            Message::OpenFile(p) => self.open_file(p),
            Message::ToggleDir(p) => {
                if let Some(root) = &mut self.root {
                    root.toggle(&p);
                }
            }
            Message::SelectTab { pane, tab } => {
                self.scm.open = false;
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
                if !self.confirm_discard(&all) {
                    return Task::none();
                }
                // Other repositories' unsaved files count too.
                let parked = self.parked_dirty_docs();
                if !parked.is_empty() {
                    let body = format!(
                        "Unsaved files in other repositories:\n{}\n\nYes saves them all, No discards the changes.",
                        parked.join("\n")
                    );
                    match ask_unsaved(&body, "Save All") {
                        Unsaved::Cancel => return Task::none(),
                        Unsaved::Discard => {}
                        Unsaved::Save => {
                            if let Err(e) = self.save_parked_docs() {
                                self.status = e;
                                return Task::none();
                            }
                        }
                    }
                }
                return iced::exit();
            }
            Message::FocusNextPane => self.panes.focus_next(),
            Message::FocusPane(i) => self.panes.focus_pane(i),
            Message::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Message::OpenMenu(i) => {
                // Clicking the open menu's button again closes it.
                self.menu = if self.menu == Some(i) { None } else { Some(i) };
                self.submenu = 0;
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
                    self.load_blame(self.panes.cur());
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
                    ("g", true) => return self.update(Message::Scm(ScmMsg::Show)),
                    ("0", true) | (")", _) => return self.update(Message::ResetWindow),
                    ("-", _) | ("_", _) => return self.update(Message::Zoom(-1)),
                    ("=", _) | ("+", _) => return self.update(Message::Zoom(1)),
                    ("0", false) => return self.update(Message::Zoom(0)),
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
        let line_h = self.panes.list[self.panes.focus].current().map(|t| t.scroll.metrics().line_h).unwrap_or(doc::LINE_H);
        let Some(d) = self.docs.get_mut(self.panes.cur()) else {
            return Task::none();
        };
        let hl = &mut self.highlighter;

        if let Key::Named(named) = key.as_ref() {
            let page = (vp.height / line_h) as isize - 1;
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

    /// Bring open files up to date with the disk, so an agent's edits show as
    /// they happen. A file with unsaved edits is left alone.
    fn reload_changed_files(&mut self) {
        let mut reloaded = Vec::new();
        let mut conflicts = Vec::new();
        for d in self.docs.iter_mut() {
            let Some(path) = d.path.clone() else { continue };
            let now = doc::disk_stamp(&path);
            if now.is_none() || now == d.stamp {
                continue;
            }
            if d.dirty {
                d.stamp = now;
                conflicts.push(d.name());
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                if text != d.buffer.text() {
                    d.replace_text(&text, &mut self.highlighter);
                    reloaded.push(d.name());
                } else {
                    d.stamp = now;
                }
            }
        }
        if let Some(name) = conflicts.first() {
            self.status = format!("{name} changed on disk, but you have unsaved edits; saving will overwrite it");
        }
        if !reloaded.is_empty() {
            self.status = format!("{} updated from disk", reloaded.join(", "));
            self.refresh_git();
        }
    }

    /// While dragging a list item, the pointer reached another one: take its place.
    fn item_enter(&mut self, target: Item) {
        match (self.item_drag.clone(), target) {
            (Some(Item::Rail(from)), Item::Rail(to)) if from != to && from.max(to) < self.projects.len() => {
                let active = self.active_project.map(|i| self.projects[i].path.clone());
                state::reorder(&mut self.projects, from, to);
                self.active_project = active.and_then(|a| self.projects.iter().position(|p| p.path == a));
                self.persisted.projects = self.projects.iter().map(|p| p.path.clone()).collect();
                self.persisted.save();
                self.item_drag = Some(Item::Rail(to));
            }
            (Some(Item::Term(a)), Item::Term(b)) if a != b => self.terms.move_session(a, b),
            (Some(Item::Pin(a)), Item::Pin(b)) if a != b => {
                let pins = &mut self.persisted.pinned;
                if let (Some(from), Some(to)) = (pins.iter().position(|p| *p == a), pins.iter().position(|p| *p == b)) {
                    state::reorder(pins, from, to);
                    self.persisted.save();
                }
            }
            _ => {}
        }
    }

    fn start_drag(&mut self, handle: Handle) {
        let mut base = (1.0, 1.0, 1.0);
        if let Handle::Panes(i) = handle {
            if let (Some(a), Some(b)) = (self.panes.list.get(i), self.panes.list.get(i + 1)) {
                let px = |p: &panes::Pane| match self.panes.axis {
                    Axis::Horizontal => p.viewport.get().width,
                    Axis::Vertical => p.viewport.get().height,
                };
                base = (a.weight, b.weight, (px(a) + px(b)).max(1.0));
            }
        }
        self.drag = Some(Drag { handle, origin: None, base });
    }

    fn drag_to(&mut self, p: iced::Point) {
        let Some(mut drag) = self.drag else { return };
        let sidebar = if self.show_sidebar { view::RAIL_W + self.list_w + view::HANDLE_W } else { 0.0 };
        let win = self.win;
        // What is docked beside the editor on each side, handles included.
        let chat_on = self.chat_visible();
        let chat_l = if chat_on && !self.chat_right { self.chat_w + view::HANDLE_W } else { 0.0 };
        let chat_r = if chat_on && self.chat_right { self.chat_w + view::HANDLE_W } else { 0.0 };
        let agent_l = if self.show_term && !self.agent_right { self.agent_w + view::HANDLE_W } else { 0.0 };
        let left = sidebar + chat_l + agent_l;
        match drag.handle {
            Handle::List => self.list_w = (p.x - view::RAIL_W).clamp(140.0, (win.width - 300.0).clamp(140.0, 480.0)),
            Handle::Agent if self.agent_right => {
                self.agent_w = (win.width - chat_r - p.x).clamp(220.0, (win.width - sidebar - 300.0).max(220.0))
            }
            Handle::Agent => {
                self.agent_w = (p.x - sidebar - chat_l).clamp(220.0, (win.width - sidebar - 300.0).max(220.0))
            }
            Handle::Chat if self.chat_right => {
                self.chat_w = (win.width - p.x).clamp(130.0, (win.width - sidebar - 300.0).clamp(130.0, 600.0))
            }
            Handle::Chat => self.chat_w = (p.x - sidebar).clamp(130.0, (win.width - sidebar - 300.0).clamp(130.0, 600.0)),
            Handle::Dock => self.agent_right = p.x > win.width / 2.0,
            Handle::ChatDock => self.chat_right = p.x > win.width / 2.0,
            Handle::Scm => self.scm_w = (p.x - left).clamp(220.0, (win.width - left - 300.0).clamp(220.0, 700.0)),
            Handle::Panes(i) => {
                let pos = match self.panes.axis {
                    Axis::Horizontal => p.x,
                    Axis::Vertical => p.y,
                };
                let origin = *drag.origin.get_or_insert(pos);
                let (wa, wb, px) = drag.base;
                let total = wa + wb;
                let a = (wa + (pos - origin) / px * total).clamp(total * 0.1, total * 0.9);
                if let Some(pane) = self.panes.list.get_mut(i) {
                    pane.weight = a;
                }
                if let Some(pane) = self.panes.list.get_mut(i + 1) {
                    pane.weight = total - a;
                }
                self.drag = Some(drag);
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            keyboard::listen().map(Message::Key),
            window::close_requests().map(|_| Message::CloseRequested),
            window::resize_events().map(|(_, size)| Message::WindowSized(size)),
        ];
        if self.chat_visible() {
            subs.push(iced::time::every(std::time::Duration::from_secs(15)).map(|_| Message::Chat(ChatMsg::Sync)));
        }
        if !self.docs.is_empty() {
            subs.push(iced::time::every(std::time::Duration::from_millis(400)).map(|_| Message::DiskTick));
        }
        if self.root.is_some() {
            subs.push(iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::TreeTick));
        }
        if self.drag.is_some() || self.tab_drag.is_some() || self.item_drag.is_some() {
            subs.push(iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => Some(Message::PointerMoved(position)),
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => Some(Message::PointerUp),
                _ => None,
            }));
        }
        // Collaboration: poll the connection for edits and cursors.
        if self.collab.is_some() {
            subs.push(iced::time::every(std::time::Duration::from_millis(40)).map(|_| Message::Collab(CollabMsg::Tick)));
        }
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
