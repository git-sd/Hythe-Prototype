//! Remembered projects and pinned files, and a quick look at the git repository.

use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_RECENTS: usize = 8;

/// What we keep between runs: recently opened project folders and pinned files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persisted {
    pub recents: Vec<PathBuf>,
    pub pinned: Vec<PathBuf>,
}

impl Persisted {
    fn file() -> Option<PathBuf> {
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

    /// One `kind=path` per line; unknown lines are ignored.
    pub fn parse(text: &str) -> Self {
        let mut p = Persisted::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("recent", path)) if !path.is_empty() => p.recents.push(PathBuf::from(path)),
                Some(("pinned", path)) if !path.is_empty() => p.pinned.push(PathBuf::from(path)),
                _ => {}
            }
        }
        p
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for r in &self.recents {
            out.push_str(&format!("recent={}\n", r.display()));
        }
        for r in &self.pinned {
            out.push_str(&format!("pinned={}\n", r.display()));
        }
        out
    }

    /// Move `dir` to the front of the recent projects.
    pub fn remember_project(&mut self, dir: &Path) {
        self.recents.retain(|r| r != dir);
        self.recents.insert(0, dir.to_path_buf());
        self.recents.truncate(MAX_RECENTS);
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

/// Branch and uncommitted-change counts for the project's repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInfo {
    pub repo: String,
    pub branch: String,
    pub added: u32,
    pub removed: u32,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The git user name, for the account chip in the corner.
pub fn git_user() -> Option<String> {
    git(Path::new("."), &["config", "user.name"]).filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_round_trips_and_ignores_junk() {
        let mut p = Persisted::default();
        p.remember_project(Path::new("C:/work/a"));
        p.remember_project(Path::new("C:/work/b"));
        p.remember_project(Path::new("C:/work/a"));
        assert_eq!(p.recents, vec![PathBuf::from("C:/work/a"), PathBuf::from("C:/work/b")]);
        assert!(p.toggle_pin(Path::new("C:/work/a/x.rs")));
        let text = p.serialize();
        assert_eq!(Persisted::parse(&format!("{text}garbage\nrecent=\n")), p);
        assert!(!p.toggle_pin(Path::new("C:/work/a/x.rs")));
    }

    #[test]
    fn recents_are_capped() {
        let mut p = Persisted::default();
        for i in 0..20 {
            p.remember_project(Path::new(&format!("/p/{i}")));
        }
        assert_eq!(p.recents.len(), MAX_RECENTS);
        assert_eq!(p.recents[0], PathBuf::from("/p/19"));
    }
}
