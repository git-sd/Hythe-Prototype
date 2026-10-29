//! What we remember between runs: the repositories in the rail, which one was
//! open, and pinned files. Plus the git user's name for the account chip.

use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_PROJECTS: usize = 24;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persisted {
    /// Repositories shown in the rail, in the order they were added.
    pub projects: Vec<PathBuf>,
    /// The project that was open when the app last ran.
    pub active: Option<PathBuf>,
    pub pinned: Vec<PathBuf>,
}

impl Persisted {
    /// `HYTHE_STATE` overrides the location (used by tests and scripted runs).
    fn file() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("HYTHE_STATE") {
            return Some(PathBuf::from(p));
        }
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("hythe").join("state.txt"))
    }

    pub fn load() -> Self {
        Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(file) = Self::file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, self.serialize());
    }

    /// One `kind=path` per line; unknown lines are ignored. Older files used
    /// `recent=` for what is now `project=`.
    pub fn parse(text: &str) -> Self {
        let mut p = Persisted::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("project" | "recent", path)) if !path.is_empty() => {
                    let path = PathBuf::from(path);
                    if !p.projects.contains(&path) {
                        p.projects.push(path);
                    }
                }
                Some(("active", path)) if !path.is_empty() => p.active = Some(PathBuf::from(path)),
                Some(("pinned", path)) if !path.is_empty() => p.pinned.push(PathBuf::from(path)),
                _ => {}
            }
        }
        p
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for r in &self.projects {
            out.push_str(&format!("project={}\n", r.display()));
        }
        if let Some(a) = &self.active {
            out.push_str(&format!("active={}\n", a.display()));
        }
        for r in &self.pinned {
            out.push_str(&format!("pinned={}\n", r.display()));
        }
        out
    }

    /// Add `dir` to the rail unless it's there. Returns its index.
    pub fn add_project(&mut self, dir: &Path) -> usize {
        if let Some(i) = self.projects.iter().position(|p| p == dir) {
            return i;
        }
        if self.projects.len() >= MAX_PROJECTS {
            self.projects.remove(0);
        }
        self.projects.push(dir.to_path_buf());
        self.projects.len() - 1
    }

    pub fn remove_project(&mut self, dir: &Path) {
        self.projects.retain(|p| p != dir);
        if self.active.as_deref() == Some(dir) {
            self.active = None;
        }
        self.pinned.retain(|f| !f.starts_with(dir));
    }

    pub fn toggle_pin(&mut self, file: &Path) -> bool {
        if let Some(i) = self.pinned.iter().position(|p| p == file) {
            self.pinned.remove(i);
            false
        } else {
            self.pinned.push(file.to_path_buf());
            true
        }
    }
}

/// Branch and uncommitted-change counts for the open project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInfo {
    pub repo: String,
    pub branch: String,
    pub added: u32,
    pub removed: u32,
}

/// The git user name, for the account chip in the corner.
pub fn git_user() -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["config", "user.name"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_keep_their_order_and_round_trip() {
        let mut p = Persisted::default();
        assert_eq!(p.add_project(Path::new("C:/work/a")), 0);
        assert_eq!(p.add_project(Path::new("C:/work/b")), 1);
        assert_eq!(p.add_project(Path::new("C:/work/a")), 0, "no duplicates, no reordering");
        p.active = Some(PathBuf::from("C:/work/b"));
        assert!(p.toggle_pin(Path::new("C:/work/a/x.rs")));
        let text = p.serialize();
        assert_eq!(Persisted::parse(&format!("{text}garbage\nproject=\n")), p);
        assert!(!p.toggle_pin(Path::new("C:/work/a/x.rs")));
    }

    #[test]
    fn old_state_files_still_load() {
        let p = Persisted::parse("recent=C:/old\nrecent=C:/older\nrecent=C:/old\n");
        assert_eq!(p.projects, vec![PathBuf::from("C:/old"), PathBuf::from("C:/older")]);
    }

    #[test]
    fn removing_a_project_forgets_its_pins_and_active_marker() {
        let mut p = Persisted::default();
        p.add_project(Path::new("/p/a"));
        p.add_project(Path::new("/p/b"));
        p.active = Some(PathBuf::from("/p/a"));
        p.toggle_pin(Path::new("/p/a/f.rs"));
        p.toggle_pin(Path::new("/p/b/g.rs"));
        p.remove_project(Path::new("/p/a"));
        assert_eq!(p.projects, vec![PathBuf::from("/p/b")]);
        assert_eq!(p.active, None);
        assert_eq!(p.pinned, vec![PathBuf::from("/p/b/g.rs")]);
    }

    #[test]
    fn the_rail_is_capped() {
        let mut p = Persisted::default();
        for i in 0..40 {
            p.add_project(Path::new(&format!("/p/{i}")));
        }
        assert_eq!(p.projects.len(), MAX_PROJECTS);
        assert_eq!(p.projects.last(), Some(&PathBuf::from("/p/39")));
    }
}
