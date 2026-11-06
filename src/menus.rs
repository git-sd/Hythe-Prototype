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
    #[allow(dead_code)]
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
            item("Full Screen", "F11", Message::ToggleFullscreen),
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
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_menus_have_valid_entries() {
        let agents = vec!["Claude Code".to_string(), "Gemini CLI".to_string()];
        for i in 0..MENUS.len() {
            let list = entries(i, &agents);
            assert!(!list.is_empty(), "menu {i} ({}) should not be empty", MENUS[i]);
            let items: Vec<_> = list.iter().filter_map(|e| match e {
                Entry::Item(it) => Some(it),
                _ => None,
            }).collect();
            assert!(!items.is_empty(), "menu {i} should contain items");
        }
    }

    #[test]
    fn terminal_menu_lists_agents() {
        let agents = vec!["Claude Code".to_string(), "Gemini CLI".to_string()];
        let list = entries(3, &agents);
        let run_claude = list.iter().any(|e| match e {
            Entry::Item(it) => it.label == "Run Claude Code" && matches!(it.message, Message::NewTerminal(Some(0))),
            _ => false,
        });
        assert!(run_claude, "terminal menu should list installed agents");
    }

    #[test]
    fn all_entries_includes_headers_and_separators() {
        let list = all_entries(&[]);
        let header_count = list.iter().filter(|e| matches!(e, Entry::Header(_))).count();
        assert_eq!(header_count, MENUS.len());
    }
}
