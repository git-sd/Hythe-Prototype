use crate::add_repo::AddRepoMsg;
use crate::panes::Axis;
use crate::scm::ScmMsg;
use crate::Message;

pub const MENUS: [&str; 4] = ["File", "Edit", "View", "Terminal"];

pub struct Item {
    pub label: String,
    pub shortcut: &'static str,
    pub message: Message,
}

pub enum Entry {
    Item(Item),
    Separator,
    /// A group title in the combined menu.
    Header(&'static str),
}

fn item(label: &str, shortcut: &'static str, message: Message) -> Entry {
    Entry::Item(Item { label: label.to_string(), shortcut, message })
}

/// Contents of one menu (an index into [`MENUS`]). `agents` are the names of
/// installed CLI agents, listed under Terminal.
pub fn entries(menu: usize, agents: &[String]) -> Vec<Entry> {
    use Entry::Separator;
    match menu {
        0 => vec![
            item("New File", "Ctrl+N", Message::NewFile),
            item("Open File…", "Ctrl+O", Message::OpenFileDialog),
            item("Open Folder…", "Ctrl+Shift+O", Message::OpenFolderDialog),
            item("Add Repository…", "", Message::AddRepo(AddRepoMsg::Open)),
            Separator,
            item("Save", "Ctrl+S", Message::Save),
            item("Close Tab", "Ctrl+W", Message::CloseActive),
            item("Pin / Unpin File", "", Message::TogglePin),
            item("Close Repository", "", Message::CloseProject),
            Separator,
            item("Exit", "", Message::CloseRequested),
        ],
        1 => vec![
            item("Undo", "Ctrl+Z", Message::Undo),
            item("Redo", "Ctrl+Y", Message::Redo),
            Separator,
            item("Select All", "Ctrl+A", Message::SelectAll),
            Separator,
            item("Show Suggestions", "Ctrl+Space", Message::TriggerCompletion),
        ],
        2 => vec![
            item("Command Palette…", "Ctrl+Shift+P", Message::OpenPalette),
            item("Source Control", "Ctrl+Shift+G", Message::Scm(ScmMsg::Show)),
            item("Toggle Inline Blame", "", Message::Scm(ScmMsg::ToggleBlame)),
            Separator,
            item("Toggle Sidebar", "Ctrl+B", Message::ToggleSidebar),
            item("Toggle Terminal", "Ctrl+`", Message::ToggleTerminal),
            Separator,
            item("Split Right", "Ctrl+\\", Message::Split(Axis::Horizontal)),
            item("Split Down", "Ctrl+Shift+\\", Message::Split(Axis::Vertical)),
            item("Close Pane", "", Message::ClosePane),
            item("Focus Next Pane", "Ctrl+1-9", Message::FocusNextPane),
            Separator,
            item("Reset Window Size", "Ctrl+Shift+0", Message::ResetWindow),
            item("Maximize / Restore Window", "", Message::ToggleMaximize),
        ],
        _ => {
            let mut v = vec![
                item("New Terminal", "Ctrl+Shift+`", Message::NewTerminal(None)),
                item("Close Terminal", "Ctrl+Shift+W", Message::CloseTerminal),
            ];
            if !agents.is_empty() {
                v.push(Separator);
            }
            for (i, name) in agents.iter().enumerate() {
                v.push(item(&format!("Run {name}"), "", Message::NewTerminal(Some(i))));
            }
            v
        }
    }
}

/// Every menu in one list, each under its title (the ☰ menu in the title bar).
pub fn all_entries(agents: &[String]) -> Vec<Entry> {
    let mut out = Vec::new();
    for (i, title) in MENUS.iter().enumerate() {
        if i > 0 {
            out.push(Entry::Separator);
        }
        out.push(Entry::Header(title));
        out.extend(entries(i, agents));
    }
    out
}

/// The "…" menu on the thread column: terminals and agents.
pub fn thread_entries(agents: &[String]) -> Vec<Entry> {
    let mut v = vec![
        item("New Terminal", "Ctrl+Shift+`", Message::NewTerminal(None)),
        item("Close Terminal", "Ctrl+Shift+W", Message::CloseTerminal),
        item("Hide Thread Panel", "Ctrl+`", Message::ToggleTerminal),
    ];
    if !agents.is_empty() {
        v.push(Entry::Separator);
    }
    for (i, name) in agents.iter().enumerate() {
        v.push(item(&format!("Run {name}"), "", Message::NewTerminal(Some(i))));
    }
    v
}

/// The "…" menu on the editor tabs: panes and file actions.
pub fn editor_entries() -> Vec<Entry> {
    use Entry::Separator;
    vec![
        item("Save", "Ctrl+S", Message::Save),
        item("Close Tab", "Ctrl+W", Message::CloseActive),
        item("Pin / Unpin File", "", Message::TogglePin),
        Separator,
        item("Split Right", "Ctrl+\\", Message::Split(Axis::Horizontal)),
        item("Split Down", "Ctrl+Shift+\\", Message::Split(Axis::Vertical)),
        item("Close Pane", "", Message::ClosePane),
        Separator,
        item("Source Control", "Ctrl+Shift+G", Message::Scm(ScmMsg::Show)),
        item("Toggle Inline Blame", "", Message::Scm(ScmMsg::ToggleBlame)),
    ]
}

/// The menu on the repository name at the top of the file list.
pub fn project_entries() -> Vec<Entry> {
    use Entry::Separator;
    vec![
        item("Add Repository…", "", Message::AddRepo(AddRepoMsg::Open)),
        item("Open Folder…", "Ctrl+Shift+O", Message::OpenFolderDialog),
        Separator,
        item("Source Control", "Ctrl+Shift+G", Message::Scm(ScmMsg::Show)),
        item("Close Repository", "", Message::CloseProject),
    ]
}