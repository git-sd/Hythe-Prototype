//! A small wrapper over the `git` command line.
//!
//! Using the real `git` keeps behaviour identical to what developers see in
//! their terminal (hooks, credentials, config) and avoids linking a Git
//! implementation. Every call returns `Result<_, String>` with git's own
//! error text.

use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, String>;

/// A git repository, found from any directory inside it.
#[derive(Debug, Clone)]
pub struct Repo {
    root: PathBuf,
}

/// What happened to a file, in the index or the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl Status {
    /// One-letter tag as shown in source control lists.
    pub fn letter(self) -> char {
        match self {
            Status::Modified => 'M',
            Status::Added => 'A',
            Status::Deleted => 'D',
            Status::Renamed => 'R',
            Status::Copied => 'C',
            Status::TypeChanged => 'T',
            Status::Untracked => 'U',
            Status::Conflicted => '!',
        }
    }

    fn from_code(c: u8) -> Option<Status> {
        Some(match c {
            b'M' => Status::Modified,
            b'A' => Status::Added,
            b'D' => Status::Deleted,
            b'R' => Status::Renamed,
            b'C' => Status::Copied,
            b'T' => Status::TypeChanged,
            b'U' => Status::Conflicted,
            b'?' => Status::Untracked,
            _ => return None,
        })
    }
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Path relative to the repository root, with `/` separators.
    pub path: String,
    /// Status of the staged (index) side, if any.
    pub staged: Option<Status>,
    /// Status of the working-tree side, if any.
    pub unstaged: Option<Status>,
    pub added: u32,
    pub removed: u32,
}

impl Change {
    pub fn is_conflicted(&self) -> bool {
        self.staged == Some(Status::Conflicted) || self.unstaged == Some(Status::Conflicted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Header,
    Hunk,
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    /// Line number in the old file (context and removed lines).
    pub old: Option<u32>,
    /// Line number in the new file (context and added lines).
    pub new: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
}

/// Who last changed one line of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub commit: String,
    pub author: String,
    /// Unix time of the commit.
    pub time: i64,
    pub summary: String,
    /// The line isn't committed yet.
    pub uncommitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contributor {
    pub name: String,
    pub email: String,
    pub commits: u32,
}

fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(["-c", "core.quotepath=false", "-c", "color.ui=never"]).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
        Err(msg)
    }
}

impl Repo {
    /// The repository containing `dir`, or `None`.
    pub fn open(dir: &Path) -> Option<Repo> {
        let top = run(dir, &["rev-parse", "--show-toplevel"]).ok()?;
        Some(Repo { root: PathBuf::from(top.trim()) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        run(&self.root, args)
    }

    // --- status and diffs ---

    pub fn current_branch(&self) -> String {
        self.git(&["rev-parse", "--abbrev-ref", "HEAD"])
            .ok()
            .map(|b| b.trim().to_string())
            .filter(|b| b != "HEAD" && !b.is_empty())
            .or_else(|| self.git(&["rev-parse", "--short", "HEAD"]).ok().map(|s| s.trim().to_string()))
            .unwrap_or_else(|| "(no commits)".into())
    }

    /// Changed files with line counts, staged and unstaged merged per file.
    pub fn status(&self) -> Result<Vec<Change>> {
        let raw = self.git(&["status", "--porcelain=v1", "-z", "-uall"])?;
        let mut changes = parse_status(&raw);

        let mut apply = |numstat: Result<String>| {
            if let Ok(text) = numstat {
                for line in text.lines() {
                    let mut cols = line.splitn(3, '\t');
                    let (Some(a), Some(r), Some(path)) = (cols.next(), cols.next(), cols.next()) else {
                        continue;
                    };
                    if let Some(c) = changes.iter_mut().find(|c| c.path == path) {
                        c.added += a.parse().unwrap_or(0);
                        c.removed += r.parse().unwrap_or(0);
                    }
                }
            }
        };
        apply(self.git(&["diff", "--numstat"]));
        apply(self.git(&["diff", "--numstat", "--cached"]));

        // Untracked files count as all-added.
        for c in changes.iter_mut().filter(|c| c.unstaged == Some(Status::Untracked)) {
            c.added = std::fs::read_to_string(self.root.join(&c.path)).map(|t| t.lines().count() as u32).unwrap_or(0);
        }
        Ok(changes)
    }

    /// Total added and removed lines against `HEAD`, for the status bar.
    pub fn summary(&self) -> (u32, u32) {
        let text = self.git(&["diff", "--numstat", "HEAD"]).unwrap_or_default();
        text.lines().fold((0, 0), |(a, r), line| {
            let mut cols = line.split('\t');
            (
                a + cols.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0),
                r + cols.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0),
            )
        })
    }

    /// Diff of one file: staged (index vs HEAD) or not (working tree vs index).
    pub fn diff(&self, path: &str, staged: bool) -> Result<Vec<DiffLine>> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-U3"];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", path]);
        let text = self.git(&args)?;
        if text.trim().is_empty() && !staged {
            // Untracked: show the whole file as added.
            if let Ok(content) = std::fs::read_to_string(self.root.join(path)) {
                let mut lines = vec![DiffLine { kind: LineKind::Hunk, text: format!("@@ new file @@ {path}"), old: None, new: None }];
                for (i, l) in content.lines().enumerate() {
                    lines.push(DiffLine { kind: LineKind::Added, text: l.to_string(), old: None, new: Some(i as u32 + 1) });
                }
                return Ok(lines);
            }
        }
        Ok(parse_diff(&text))
    }

    // --- staging and commits ---

    pub fn stage(&self, paths: &[&str]) -> Result<()> {
        let mut args = vec!["add", "--"];
        args.extend(paths);
        self.git(&args).map(|_| ())
    }

    pub fn unstage(&self, paths: &[&str]) -> Result<()> {
        // `restore --staged` fails before the first commit; `rm --cached` handles that case.
        let mut args = vec!["restore", "--staged", "--"];
        args.extend(paths);
        match self.git(&args) {
            Ok(_) => Ok(()),
            Err(_) => {
                let mut args = vec!["rm", "--cached", "-q", "--"];
                args.extend(paths);
                self.git(&args).map(|_| ())
            }
        }
    }

    pub fn commit(&self, message: &str) -> Result<String> {
        if message.trim().is_empty() {
            return Err("write a commit message first".into());
        }
        self.git(&["commit", "-m", message])?;
        self.git(&["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string())
    }

    // --- remotes ---

    pub fn push(&self) -> Result<String> {
        match self.git(&["push"]) {
            Ok(out) => Ok(out),
            // A new branch has no upstream yet.
            Err(e) if e.contains("no upstream") || e.contains("--set-upstream") => {
                let branch = self.current_branch();
                self.git(&["push", "--set-upstream", "origin", &branch])
            }
            Err(e) => Err(e),
        }
    }

    pub fn pull(&self) -> Result<String> {
        self.git(&["pull", "--no-rebase"])
    }

    pub fn fetch(&self) -> Result<String> {
        self.git(&["fetch", "--prune"])
    }

    /// Commits the local branch is (ahead, behind) its upstream.
    pub fn ahead_behind(&self) -> Option<(u32, u32)> {
        let out = self.git(&["rev-list", "--left-right", "--count", "HEAD...@{upstream}"]).ok()?;
        let mut it = out.split_whitespace();
        Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    }

    // --- branches ---

    pub fn branches(&self) -> Result<Vec<Branch>> {
        let out = self.git(&["branch", "--format=%(HEAD)%(refname:short)"])?;
        Ok(out
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.contains("HEAD detached"))
            .map(|l| Branch { current: l.starts_with('*'), name: l.trim_start_matches(['*', ' ']).to_string() })
            .collect())
    }

    pub fn checkout(&self, branch: &str) -> Result<()> {
        self.git(&["checkout", branch]).map(|_| ())
    }

    pub fn create_branch(&self, name: &str) -> Result<()> {
        if name.trim().is_empty() || name.contains(char::is_whitespace) {
            return Err("branch names can't be empty or contain spaces".into());
        }
        self.git(&["checkout", "-b", name]).map(|_| ())
    }

    // --- history ---

    /// Last-change info for every line of `path` (working-tree version).
    pub fn blame(&self, path: &str) -> Result<Vec<BlameLine>> {
        let out = self.git(&["blame", "--line-porcelain", "--", path])?;
        Ok(parse_blame(&out))
    }

    /// People who committed, most active first.
    pub fn contributors(&self) -> Vec<Contributor> {
        let out = self.git(&["shortlog", "-sne", "HEAD"]).unwrap_or_default();
        out.lines()
            .filter_map(|l| {
                let l = l.trim();
                let (count, rest) = l.split_once('\t')?;
                let (name, email) = rest.rsplit_once(" <")?;
                Some(Contributor {
                    name: name.trim().to_string(),
                    email: email.trim_end_matches('>').to_string(),
                    commits: count.trim().parse().ok()?,
                })
            })
            .collect()
    }
}

/// What to pass to `git clone` for what a person typed: `owner/name` and
/// `github.com/owner/name` become GitHub URLs; anything else is used as given.
pub fn clone_url(input: &str) -> String {
    let s = input.trim();
    let is_name = |seg: &str| !seg.is_empty() && seg != "." && seg != ".." && seg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if s.contains("://") || s.starts_with("git@") || s.contains(':') || s.contains('\\') {
        return s.to_string();
    }
    let path = s.strip_prefix("github.com/").unwrap_or(s);
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if segs.len() == 2 && segs.iter().all(|seg| is_name(seg)) {
        let repo = segs[1].trim_end_matches(".git");
        return format!("https://github.com/{}/{}.git", segs[0], repo);
    }
    s.to_string()
}

/// Folder name `git clone` would pick for `url`: the last segment without `.git`.
pub fn repo_dir_name(url: &str) -> String {
    let u = url.trim().trim_end_matches(['/', '\\']);
    let last = u.rsplit(['/', '\\', ':']).next().unwrap_or(u);
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

/// Clone `url` into `dest` (which must not exist yet, or be empty).
pub fn clone_repo(url: &str, dest: &Path) -> Result<()> {
    if dest.exists() && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} already exists and isn't empty", dest.display()));
    }
    let parent = dest.parent().ok_or("choose a folder to clone into")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let name = dest.file_name().ok_or("choose a folder to clone into")?.to_string_lossy().into_owned();
    run(parent, &["clone", "--", url, &name]).map(|_| ())
}
/// Parse `git status --porcelain=v1 -z` output.
pub fn parse_status(raw: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0').filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        let b = entry.as_bytes();
        if b.len() < 4 {
            continue;
        }
        let (x, y) = (b[0], b[1]);
        let path = entry[3..].to_string();
        // Renames and copies are followed by the original path.
        if x == b'R' || x == b'C' || y == b'R' || y == b'C' {
            parts.next();
        }
        let conflicted = matches!((x, y), (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D'));
        let (staged, unstaged) = if x == b'?' {
            (None, Some(Status::Untracked))
        } else if conflicted {
            (Some(Status::Conflicted), None)
        } else {
            (Status::from_code(x), Status::from_code(y))
        };
        out.push(Change { path, staged, unstaged, added: 0, removed: 0 });
    }
    out
}

/// Parse a unified diff into display lines with line numbers.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            // "@@ -a,b +c,d @@ context"
            let nums: Vec<u32> = rest
                .split_whitespace()
                .take(2)
                .filter_map(|p| p.trim_start_matches(['-', '+']).split(',').next()?.parse().ok())
                .collect();
            if nums.len() == 2 {
                old = nums[0];
                new = nums[1];
            }
            in_hunk = true;
            out.push(DiffLine { kind: LineKind::Hunk, text: format!("@@{rest}"), old: None, new: None });
        } else if !in_hunk {
            if line.starts_with("Binary files") {
                out.push(DiffLine { kind: LineKind::Header, text: line.to_string(), old: None, new: None });
            }
            // Skip `diff --git`, `index`, `---`, `+++` preamble.
        } else if let Some(t) = line.strip_prefix('+') {
            out.push(DiffLine { kind: LineKind::Added, text: t.to_string(), old: None, new: Some(new) });
            new += 1;
        } else if let Some(t) = line.strip_prefix('-') {
            out.push(DiffLine { kind: LineKind::Removed, text: t.to_string(), old: Some(old), new: None });
            old += 1;
        } else if line.starts_with('\\') {
            // "\ No newline at end of file"
        } else {
            let t = line.strip_prefix(' ').unwrap_or(line);
            out.push(DiffLine { kind: LineKind::Context, text: t.to_string(), old: Some(old), new: Some(new) });
            old += 1;
            new += 1;
        }
    }
    out
}

/// Parse `git blame --line-porcelain` output into one entry per final line.
pub fn parse_blame(text: &str) -> Vec<BlameLine> {
    let mut out = Vec::new();
    let mut cur: Option<BlameLine> = None;
    for line in text.lines() {
        if line.starts_with('\t') {
            if let Some(b) = cur.take() {
                out.push(b);
            }
        } else if let Some(b) = cur.as_mut() {
            if let Some(a) = line.strip_prefix("author ") {
                b.author = a.to_string();
            } else if let Some(t) = line.strip_prefix("author-time ") {
                b.time = t.parse().unwrap_or(0);
            } else if let Some(s) = line.strip_prefix("summary ") {
                b.summary = s.to_string();
            }
        } else {
            // "<sha> <orig line> <final line> [<group size>]"
            let sha = line.split_whitespace().next().unwrap_or("");
            if sha.len() >= 40 {
                cur = Some(BlameLine {
                    commit: sha[..8].to_string(),
                    author: String::new(),
                    time: 0,
                    summary: String::new(),
                    uncommitted: sha.bytes().all(|c| c == b'0'),
                });
            }
        }
    }
    out
}

/// "3 minutes ago", "2 days ago" for a unix timestamp.
pub fn time_ago(then: i64, now: i64) -> String {
    let secs = (now - then).max(0);
    let (n, unit) = match secs {
        0..=59 => return "just now".into(),
        60..=3599 => (secs / 60, "minute"),
        3600..=86399 => (secs / 3600, "hour"),
        86400..=2_591_999 => (secs / 86400, "day"),
        2_592_000..=31_535_999 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A throwaway repository with a committed `a.txt`.
    struct Fixture {
        dir: PathBuf,
        repo: Repo,
    }

    impl Fixture {
        fn new() -> Fixture {
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!("hythe-git-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            run(&dir, &["init", "-q", "-b", "main"]).unwrap();
            run(&dir, &["config", "user.name", "Test User"]).unwrap();
            run(&dir, &["config", "user.email", "test@example.com"]).unwrap();
            run(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
            std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
            run(&dir, &["add", "a.txt"]).unwrap();
            run(&dir, &["commit", "-q", "-m", "first commit"]).unwrap();
            let repo = Repo::open(&dir).expect("repo");
            Fixture { dir, repo }
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.dir.join(name), text).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn open_finds_the_repo_from_a_subdirectory() {
        let f = Fixture::new();
        std::fs::create_dir_all(f.dir.join("sub")).unwrap();
        let again = Repo::open(&f.dir.join("sub")).unwrap();
        assert_eq!(again.name(), f.repo.name());
        assert!(Repo::open(&std::env::temp_dir().join("definitely-not-a-repo-xyz")).is_none());
    }

    #[test]
    fn status_reports_modified_untracked_and_counts() {
        let f = Fixture::new();
        f.write("a.txt", "one\nTWO\nthree\nfour\n");
        f.write("new.txt", "x\ny\n");
        let changes = f.repo.status().unwrap();
        let a = changes.iter().find(|c| c.path == "a.txt").unwrap();
        assert_eq!((a.staged, a.unstaged), (None, Some(Status::Modified)));
        assert_eq!((a.added, a.removed), (2, 1));
        let n = changes.iter().find(|c| c.path == "new.txt").unwrap();
        assert_eq!(n.unstaged, Some(Status::Untracked));
        assert_eq!(n.added, 2);
        assert_eq!(f.repo.summary(), (2, 1));
    }

    #[test]
    fn diff_has_line_numbers_and_handles_untracked_files() {
        let f = Fixture::new();
        f.write("a.txt", "one\nTWO\nthree\n");
        let d = f.repo.diff("a.txt", false).unwrap();
        assert_eq!(d[0].kind, LineKind::Hunk);
        let removed = d.iter().find(|l| l.kind == LineKind::Removed).unwrap();
        assert_eq!((removed.text.as_str(), removed.old), ("two", Some(2)));
        let added = d.iter().find(|l| l.kind == LineKind::Added).unwrap();
        assert_eq!((added.text.as_str(), added.new), ("TWO", Some(2)));
        assert!(d.iter().any(|l| l.kind == LineKind::Context && l.text == "one" && l.new == Some(1)));

        f.write("new.txt", "hello\nworld\n");
        let d = f.repo.diff("new.txt", false).unwrap();
        assert_eq!(d.iter().filter(|l| l.kind == LineKind::Added).count(), 2);
    }

    #[test]
    fn stage_unstage_and_commit() {
        let f = Fixture::new();
        f.write("a.txt", "one\ntwo\nthree\nfour\n");
        f.repo.stage(&["a.txt"]).unwrap();
        let c = &f.repo.status().unwrap()[0];
        assert_eq!((c.staged, c.unstaged), (Some(Status::Modified), None));
        assert_eq!(f.repo.diff("a.txt", true).unwrap().iter().filter(|l| l.kind == LineKind::Added).count(), 1);

        f.repo.unstage(&["a.txt"]).unwrap();
        assert_eq!(f.repo.status().unwrap()[0].staged, None);

        assert!(f.repo.commit("   ").is_err());
        f.repo.stage(&["a.txt"]).unwrap();
        let sha = f.repo.commit("add four").unwrap();
        assert!(!sha.is_empty());
        assert!(f.repo.status().unwrap().is_empty());
    }

    #[test]
    fn branches_can_be_created_and_switched() {
        let f = Fixture::new();
        assert_eq!(f.repo.current_branch(), "main");
        f.repo.create_branch("feature/x").unwrap();
        assert_eq!(f.repo.current_branch(), "feature/x");
        let names: Vec<_> = f.repo.branches().unwrap().into_iter().map(|b| (b.name, b.current)).collect();
        assert!(names.contains(&("main".to_string(), false)));
        assert!(names.contains(&("feature/x".to_string(), true)));
        f.repo.checkout("main").unwrap();
        assert_eq!(f.repo.current_branch(), "main");
        assert!(f.repo.create_branch("has space").is_err());
        assert!(f.repo.checkout("nope").is_err());
    }

    #[test]
    fn blame_attributes_committed_and_uncommitted_lines() {
        let f = Fixture::new();
        f.write("a.txt", "one\ntwo CHANGED\nthree\n");
        let b = f.repo.blame("a.txt").unwrap();
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].author, "Test User");
        assert_eq!(b[0].summary, "first commit");
        assert!(!b[0].uncommitted);
        assert!(b[1].uncommitted, "edited line isn't committed: {:?}", b[1]);
    }

    #[test]
    fn contributors_are_listed() {
        let f = Fixture::new();
        let c = f.repo.contributors();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].name.as_str(), c[0].email.as_str(), c[0].commits), ("Test User", "test@example.com", 1));
    }

    #[test]
    fn parses_renames_and_conflicts() {
        let raw = "R  new.txt\0old.txt\0UU both.txt\0?? u.txt\0 M m.txt\0A  a.txt\0";
        let c = parse_status(raw);
        assert_eq!(c.len(), 5);
        assert_eq!(c[0].staged, Some(Status::Renamed));
        assert_eq!(c[0].path, "new.txt");
        assert!(c[1].is_conflicted());
        assert_eq!(c[2].unstaged, Some(Status::Untracked));
        assert_eq!((c[3].staged, c[3].unstaged), (None, Some(Status::Modified)));
        assert_eq!(c[4].staged, Some(Status::Added));
    }

    #[test]
    fn push_pull_and_ahead_behind_against_a_bare_remote() {
        let f = Fixture::new();
        let remote = f.dir.with_extension("remote.git");
        let _ = std::fs::remove_dir_all(&remote);
        run(&f.dir, &["init", "-q", "--bare", remote.to_str().unwrap()]).unwrap();
        run(&f.dir, &["remote", "add", "origin", remote.to_str().unwrap()]).unwrap();
        f.repo.push().expect("first push sets the upstream");
        assert_eq!(f.repo.ahead_behind(), Some((0, 0)));

        f.write("a.txt", "one\ntwo\nthree\nfour\n");
        f.repo.stage(&["a.txt"]).unwrap();
        f.repo.commit("local change").unwrap();
        assert_eq!(f.repo.ahead_behind(), Some((1, 0)));
        f.repo.push().unwrap();
        assert_eq!(f.repo.ahead_behind(), Some((0, 0)));
        f.repo.fetch().unwrap();
        f.repo.pull().unwrap();
        let _ = std::fs::remove_dir_all(&remote);
    }

    #[test]
    fn clone_urls_are_expanded_from_shorthand() {
        assert_eq!(clone_url("rust-lang/rust"), "https://github.com/rust-lang/rust.git");
        assert_eq!(clone_url("  github.com/git-sd/Hythe-Prototype/ "), "https://github.com/git-sd/Hythe-Prototype.git");
        assert_eq!(clone_url("https://github.com/a/b"), "https://github.com/a/b");
        assert_eq!(clone_url("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(clone_url("C:\\work\\repo"), "C:\\work\\repo");
        assert_eq!(clone_url("single"), "single");
        assert_eq!(repo_dir_name("https://github.com/a/b.git"), "b");
        assert_eq!(repo_dir_name("git@github.com:a/b.git"), "b");
        assert_eq!(repo_dir_name("C:\\work\\repo\\"), "repo");
    }

    #[test]
    fn cloning_a_repository_copies_its_files_and_refuses_to_overwrite() {
        let f = Fixture::new();
        let dest = f.dir.with_extension("clone");
        let _ = std::fs::remove_dir_all(&dest);
        clone_repo(f.dir.to_str().unwrap(), &dest).expect("clone");
        assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap().replace("\r\n", "\n"), "one\ntwo\nthree\n");
        assert!(Repo::open(&dest).is_some());
        let again = clone_repo(f.dir.to_str().unwrap(), &dest);
        assert!(again.is_err(), "must not clone over an existing folder");
        let _ = std::fs::remove_dir_all(&dest);
    }
    #[test]
    fn time_ago_reads_naturally() {
        assert_eq!(time_ago(100, 130), "just now");
        assert_eq!(time_ago(0, 120), "2 minutes ago");
        assert_eq!(time_ago(0, 3600), "1 hour ago");
        assert_eq!(time_ago(0, 3 * 86400), "3 days ago");
        assert_eq!(time_ago(0, 400 * 86400), "1 year ago");
    }
}
