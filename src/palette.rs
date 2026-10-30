use crate::add_repo::AddRepoMsg;
use crate::panes::Axis;
use crate::scm::ScmMsg;
use crate::Message;

pub struct Palette {
    pub query: String,
    pub selected: usize,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
        }
    }
}

#[derive(Clone)]
pub struct Command {
    pub label: String,
    pub shortcut: &'static str,
    pub message: Message,
}

/// All commands. `agents` are the names of installed CLI agents, each of
/// which gets a "Run …" entry.
fn all(agents: &[String]) -> Vec<Command> {
    let c = |label: &str, shortcut, message| Command { label: label.to_string(), shortcut, message };
    let mut commands = vec![
        c("New File", "Ctrl+N", Message::NewFile),
        c("Open File", "Ctrl+O", Message::OpenFileDialog),
        c("Open Folder", "Ctrl+Shift+O", Message::OpenFolderDialog),
        c("Add Repository", "", Message::AddRepo(AddRepoMsg::Open)),
        c("Save File", "Ctrl+S", Message::Save),
        c("Close Tab", "Ctrl+W", Message::CloseActive),
        c("Undo", "Ctrl+Z", Message::Undo),
        c("Redo", "Ctrl+Y", Message::Redo),
        c("Select All", "Ctrl+A", Message::SelectAll),
        c("Show Suggestions", "Ctrl+Space", Message::TriggerCompletion),
        c("Split Right", "Ctrl+\\", Message::Split(Axis::Horizontal)),
        c("Split Down", "Ctrl+Shift+\\", Message::Split(Axis::Vertical)),
        c("Close Pane", "", Message::ClosePane),
        c("Focus Next Pane", "Ctrl+1-9", Message::FocusNextPane),
        c("Close Repository", "", Message::CloseProject),
        c("Source Control", "Ctrl+Shift+G", Message::Scm(ScmMsg::Show)),
        c("Reset Window Size", "Ctrl+Shift+0", Message::ResetWindow),
        c("Maximize / Restore Window", "", Message::ToggleMaximize),
        c("Full Screen", "F11", Message::ToggleFullscreen),
        c("Git: Push", "", Message::Scm(ScmMsg::Push)),
        c("Git: Pull", "", Message::Scm(ScmMsg::Pull)),
        c("Git: Fetch", "", Message::Scm(ScmMsg::Fetch)),
        c("Git: Switch Branch…", "", Message::Scm(ScmMsg::BranchMenu(true))),
        c("Toggle Inline Blame", "", Message::Scm(ScmMsg::ToggleBlame)),
        c("Toggle Explorer", "Ctrl+B", Message::ToggleSidebar),
        c("Toggle Terminal", "Ctrl+`", Message::ToggleTerminal),
        c("New Terminal", "Ctrl+Shift+`", Message::NewTerminal(None)),
        c("Close Terminal", "Ctrl+Shift+W", Message::CloseTerminal),
    ];
    for (i, name) in agents.iter().enumerate() {
        commands.push(c(&format!("Run {name}"), "", Message::NewTerminal(Some(i))));
    }
    commands
}

/// Subsequence match; lower score is better. `None` if `query` doesn't match.
fn fuzzy(query: &str, label: &str) -> Option<usize> {
    let label = label.to_lowercase();
    let mut chars = label.char_indices();
    let mut score = 0;
    let mut last = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let (i, _) = chars.find(|(_, c)| *c == q)?;
        score += match last {
            Some(l) => i - l - 1,
            None => i,
        };
        last = Some(i);
    }
    Some(score)
}

/// Commands matching `query`, best match first.
pub fn filter(query: &str, agents: &[String]) -> Vec<Command> {
    let mut scored: Vec<(usize, Command)> = all(agents)
        .into_iter()
        .filter_map(|c| fuzzy(query, &c.label).map(|s| (s, c)))
        .collect();
    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_filters_and_ranks() {
        assert_eq!(filter("", &[]).len(), all(&[]).len());
        assert_eq!(filter("sav", &[])[0].label, "Save File");
        assert_eq!(filter("tgl exp", &[])[0].label, "Toggle Explorer");
        assert!(filter("zzz", &[]).is_empty());
    }

    #[test]
    fn installed_agents_get_run_commands() {
        let agents = vec!["Claude Code".to_string()];
        let hit = filter("run claude", &agents);
        assert_eq!(hit[0].label, "Run Claude Code");
        assert!(matches!(hit[0].message, Message::NewTerminal(Some(0))));
    }
}
