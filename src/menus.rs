use crate::panes::Axis;
use crate::Message;

pub const MENUS: [&str; 4] = ["File", "Edit", "View", "Terminal"];
/// Width of each menu button in the top bar; the dropdown lines up using this.
pub const MENU_W: f32 = 76.0;

pub struct Item {
    pub label: String,
    pub shortcut: &'static str,
    pub message: Message,
}

pub enum Entry {
    Item(Item),
    Separator,
}

fn item(label: &str, shortcut: &'static str, message: Message) -> Entry {
    Entry::Item(Item { label: label.to_string(), shortcut, message })
}

/// Contents of dropdown `menu` (an index into [`MENUS`]). `agents` are the
/// names of installed CLI agents, listed under Terminal.
pub fn entries(menu: usize, agents: &[String]) -> Vec<Entry> {
    use Entry::Separator;
    match menu {
        0 => vec![
            item("New File", "Ctrl+N", Message::NewFile),
            item("Open File…", "Ctrl+O", Message::OpenFileDialog),
            item("Open Folder…", "Ctrl+Shift+O", Message::OpenFolderDialog),
            Separator,
            item("Save", "Ctrl+S", Message::Save),
            item("Close Tab", "Ctrl+W", Message::CloseActive),
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
            Separator,
            item("Toggle Explorer", "Ctrl+B", Message::ToggleSidebar),
            item("Toggle Terminal", "Ctrl+`", Message::ToggleTerminal),
            Separator,
            item("Split Right", "Ctrl+\\", Message::Split(Axis::Horizontal)),
            item("Split Down", "Ctrl+Shift+\\", Message::Split(Axis::Vertical)),
            item("Close Pane", "", Message::ClosePane),
            item("Focus Next Pane", "Ctrl+1-9", Message::FocusNextPane),
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
