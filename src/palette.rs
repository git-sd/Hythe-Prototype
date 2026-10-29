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
    pub label: &'static str,
    pub shortcut: &'static str,
    pub message: Message,
}

fn all() -> Vec<Command> {
    let c = |label, shortcut, message| Command { label, shortcut, message };
    vec![
        c("New File", "Ctrl+N", Message::NewFile),
        c("Open File", "Ctrl+O", Message::OpenFileDialog),
        c("Open Folder", "Ctrl+Shift+O", Message::OpenFolderDialog),
        c("Save File", "Ctrl+S", Message::Save),
        c("Close Tab", "Ctrl+W", Message::CloseActive),
        c("Undo", "Ctrl+Z", Message::Undo),
        c("Redo", "Ctrl+Y", Message::Redo),
        c("Select All", "Ctrl+A", Message::SelectAll),
        c("Toggle Explorer", "Ctrl+B", Message::ToggleSidebar),
        c("Toggle Assistant", "", Message::ToggleAi),
    ]
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
pub fn filter(query: &str) -> Vec<Command> {
    let mut scored: Vec<(usize, Command)> = all()
        .into_iter()
        .filter_map(|c| fuzzy(query, c.label).map(|s| (s, c)))
        .collect();
    scored.sort_by_key(|(s, _)| *s);
    scored.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_filters_and_ranks() {
        assert_eq!(filter("").len(), all().len());
        assert_eq!(filter("sav")[0].label, "Save File");
        assert_eq!(filter("tgl exp")[0].label, "Toggle Explorer");
        assert!(filter("zzz").is_empty());
    }
}
