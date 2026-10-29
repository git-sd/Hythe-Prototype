use std::path::{Path, PathBuf};

/// What to run in a new terminal session.
#[derive(Debug, Clone, Default)]
pub struct SpawnConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Extra environment variables, applied on top of the parent environment.
    pub env: Vec<(String, String)>,
    /// Display name for tabs; defaults to the program name.
    pub name: Option<String>,
}

impl SpawnConfig {
    pub fn new(program: impl Into<String>) -> Self {
        Self { program: program.into(), ..Default::default() }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub(crate) fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            Path::new(&self.program)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.program.clone())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKind {
    Shell,
    Agent,
}

/// A named, reusable launch configuration: a shell or a CLI coding agent.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub kind: ProfileKind,
    /// The program was found on PATH when the profile list was built.
    pub installed: bool,
}

impl Profile {
    pub fn agent(name: &str, program: &str, args: &[&str]) -> Self {
        Self {
            name: name.into(),
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            kind: ProfileKind::Agent,
            installed: false,
        }
    }

    /// The user's default shell.
    pub fn shell() -> Self {
        let program = default_shell();
        Self { name: "Shell".into(), program, args: Vec::new(), kind: ProfileKind::Shell, installed: true }
    }

    /// Whether the program can be found on `PATH`.
    pub fn is_available(&self) -> bool {
        find_in_path(&self.program).is_some()
    }

    pub fn spawn_config(&self) -> SpawnConfig {
        SpawnConfig {
            program: self.program.clone(),
            args: self.args.clone(),
            name: Some(self.name.clone()),
            ..Default::default()
        }
    }
}

/// CLI coding agents this crate knows how to launch, installed or not.
/// `installed` says whether each one was found on `PATH`.
pub fn known_agents() -> Vec<Profile> {
    [
        ("Claude Code", "claude"),
        ("Codex", "codex"),
        ("Gemini CLI", "gemini"),
        ("Cursor Agent", "cursor-agent"),
        ("Antigravity", "antigravity"),
        ("GitHub Copilot", "copilot"),
        ("Aider", "aider"),
        ("OpenCode", "opencode"),
        ("Qwen Code", "qwen"),
        ("Amp", "amp"),
        ("Goose", "goose"),
        ("Crush", "crush"),
    ]
    .into_iter()
    .map(|(name, program)| {
        let mut p = Profile::agent(name, program, &[]);
        p.installed = p.is_available();
        p
    })
    .collect()
}

/// The subset of [`known_agents`] installed on this machine.
pub fn available_agents() -> Vec<Profile> {
    known_agents().into_iter().filter(|p| p.installed).collect()
}
fn default_shell() -> String {
    #[cfg(windows)]
    {
        for candidate in ["pwsh", "powershell"] {
            if find_in_path(candidate).is_some() {
                return candidate.to_string();
            }
        }
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

/// Resolve `program` the way a shell would: as given if it has a directory
/// part, otherwise by searching `PATH` (and `PATHEXT` on Windows).
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.components().count() > 1 {
        return direct.is_file().then(|| direct.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        // Prefer a program that is already spelled with an extension.
        std::iter::once(String::new())
            .chain(pathext.split(';').filter(|e| !e.is_empty()).map(str::to_string))
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() && (!ext.is_empty() || !cfg!(windows) || Path::new(program).extension().is_some()) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_is_found() {
        assert!(Profile::shell().is_available(), "no shell on PATH: {:?}", Profile::shell());
    }

    #[test]
    fn missing_programs_are_not_available() {
        assert!(find_in_path("definitely-not-a-real-program-xyz").is_none());
        assert!(!Profile::agent("x", "definitely-not-a-real-program-xyz", &[]).is_available());
    }

    #[test]
    fn known_agents_are_unique_and_flag_what_is_installed() {
        let agents = known_agents();
        assert!(agents.len() >= 10);
        let names: std::collections::HashSet<_> = agents.iter().map(|a| a.name.clone()).collect();
        let programs: std::collections::HashSet<_> = agents.iter().map(|a| a.program.clone()).collect();
        assert_eq!(names.len(), agents.len());
        assert_eq!(programs.len(), agents.len());
        for a in &agents {
            assert_eq!(a.installed, a.is_available(), "{}", a.name);
        }
        assert!(agents.iter().any(|a| a.name == "Antigravity") && agents.iter().any(|a| a.name == "Codex"));
    }

    #[test]
    fn display_name_uses_stem() {
        assert_eq!(SpawnConfig::new("/usr/bin/zsh").display_name(), "zsh");
        assert_eq!(SpawnConfig::new("claude").name("Claude Code").display_name(), "Claude Code");
    }
}
