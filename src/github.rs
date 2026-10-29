//! The account's most recently pushed GitHub repositories, for the clone list.
//!
//! With the GitHub CLI signed in (`gh auth login`) this lists the account's own
//! repositories, private ones included. Without it, the public repositories of
//! the account's username are listed through the public API with `curl`.

use std::process::Command;

#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub full_name: String,
    pub description: String,
    pub private: bool,
    pub clone_url: String,
    /// `YYYY-MM-DD` of the last push.
    pub pushed: String,
}

fn quiet(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn output(mut cmd: Command) -> Option<String> {
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Up to ten repositories, most recently pushed first.
pub fn recent_repos(login: Option<&str>) -> Result<Vec<RepoEntry>, String> {
    let mut gh = quiet("gh");
    gh.args(["api", "user/repos?sort=pushed&direction=desc&per_page=10"]);
    if let Some(list) = output(gh).and_then(|json| parse(&json)) {
        return Ok(list);
    }
    let login = login.filter(|l| hythe_git::is_github_login(l)).ok_or("No GitHub username known")?;
    let mut curl = quiet("curl");
    curl.args(["-sSL", "-m", "10", &format!("https://api.github.com/users/{login}/repos?sort=pushed&direction=desc&per_page=10")]);
    let json = output(curl).ok_or("Could not reach GitHub")?;
    parse(&json).ok_or_else(|| format!("No repositories found for {login}"))
}

pub fn parse(json: &str) -> Option<Vec<RepoEntry>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let list = value
        .as_array()?
        .iter()
        .filter_map(|r| {
            let text = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let full_name = text("full_name");
            (!full_name.is_empty()).then(|| RepoEntry {
                full_name,
                description: text("description"),
                private: r.get("private").and_then(|v| v.as_bool()).unwrap_or(false),
                clone_url: text("clone_url"),
                pushed: text("pushed_at").chars().take(10).collect(),
            })
        })
        .collect();
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_listing() {
        let json = r#"[{"full_name":"a/b","description":null,"private":true,"clone_url":"https://github.com/a/b.git","pushed_at":"2026-10-01T12:00:00Z"}]"#;
        let list = parse(json).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].full_name, "a/b");
        assert!(list[0].private);
        assert_eq!(list[0].pushed, "2026-10-01");
        assert!(parse(r#"{"message":"Not Found"}"#).is_none());
    }
}
