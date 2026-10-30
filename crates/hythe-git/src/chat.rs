//! Repository chat kept in git itself.
//!
//! Messages live on an orphan branch, `hythe-chat`, that is never checked out:
//! each message (and each uploaded file) is one uniquely named file in the
//! branch's tree, written with plumbing commands and a throw-away index, so the
//! working tree and the current branch are never touched. Because names are
//! unique, two people's histories always merge by taking the union of both
//! trees. Sharing is a plain `fetch` and `push` of that branch.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{noreply_login, Repo, Result};

const LOCAL: &str = "refs/heads/hythe-chat";
const REMOTE: &str = "refs/remotes/origin/hythe-chat";
/// GitHub refuses files over 100 MB; stay well under.
pub const MAX_UPLOAD: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Entry name in the chat tree.
    pub entry: String,
    /// The file name it was uploaded with.
    pub name: String,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].iter().any(|e| n.ends_with(e))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Entry name; sorts oldest first.
    pub id: String,
    pub author: String,
    /// GitHub login, when known (for the profile picture).
    pub login: Option<String>,
    pub time_ms: i64,
    pub body: String,
    pub attachments: Vec<Attachment>,
    /// The message this one comments on, if it is a reply.
    pub reply_to: Option<String>,
    /// The words of that message the comment is about.
    pub quote: Option<String>,
}

/// One git invocation with extra environment and optional stdin; stdout as bytes.
fn git(dir: &Path, args: &[&str], envs: &[(&str, &Path)], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(["-c", "core.quotepath=false"]).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A thread keeps a large upload from deadlocking against git's output.
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err })
    }
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn unique() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("{:08x}", (n ^ ((std::process::id() as u64) << 20)) & 0xffff_ffff)
}

fn clean(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_') { c } else { '_' })
        .take(80)
        .collect();
    if s.is_empty() { "file".into() } else { s }
}

impl Repo {
    fn chat_git(&self, args: &[&str]) -> Result<String> {
        git(&self.root, args, &[], None).map(text)
    }

    fn chat_ref(&self, name: &str) -> Option<String> {
        self.chat_git(&["rev-parse", "--verify", "-q", name]).ok().filter(|s| !s.is_empty())
    }

    /// A scratch index in `.git`, so the real one is never involved.
    fn chat_index(&self) -> Result<PathBuf> {
        let rel = self.chat_git(&["rev-parse", "--git-path", "hythe-chat-index"])?;
        let path = if Path::new(&rel).is_absolute() { PathBuf::from(rel) } else { self.root.join(rel) };
        let _ = std::fs::remove_file(&path);
        Ok(path)
    }

    /// Who is posting: (name, GitHub login if it can be told).
    pub fn chat_identity(&self) -> (String, Option<String>) {
        let config = |key: &str| self.chat_git(&["config", "--get", key]).ok().filter(|s| !s.is_empty());
        let name = config("user.name").unwrap_or_else(|| "someone".into());
        let login = config("user.email")
            .and_then(|e| noreply_login(&e))
            .or_else(|| config("github.user").filter(|u| super::is_github_login(u)));
        (name, login)
    }

    /// Commit the files in `entries` (name, blob id) on top of the chat branch.
    fn chat_commit(&self, entries: &[(String, String)], parent: Option<&str>, extra_parent: Option<&str>) -> Result<String> {
        let index = self.chat_index()?;
        let env = [("GIT_INDEX_FILE", index.as_path())];
        let run = |args: &[&str]| git(&self.root, args, &env, None).map(text);
        if let Some(p) = parent {
            run(&["read-tree", p])?;
        }
        for (name, blob) in entries {
            run(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{name}")])?;
        }
        let tree = run(&["write-tree"])?;
        let mut args = vec!["commit-tree", tree.as_str(), "-m", "chat"];
        for p in parent.iter().chain(extra_parent.iter()) {
            args.push("-p");
            args.push(p);
        }
        let commit = git(&self.root, &args, &[], None).map(text);
        let _ = std::fs::remove_file(&index);
        commit
    }

    /// Add a message, with any files, to the local chat branch.
    /// `reply_to`: the id of the message this one comments on; `quote`: the words of it
    /// the comment is about.
    pub fn chat_post(&self, body: &str, files: &[PathBuf], reply_to: Option<&str>, quote: Option<&str>) -> Result<()> {
        let (author, login) = self.chat_identity();
        let ms = now_ms();
        let mut entries = Vec::new();
        let mut header = format!("author: {author}\n");
        if let Some(l) = &login {
            header.push_str(&format!("login: {l}\n"));
        }
        header.push_str(&format!("time: {ms}\n"));
        if let Some(id) = reply_to.filter(|id| id.starts_with("m-") && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))) {
            header.push_str(&format!("reply: {id}\n"));
            // One line, so it stays one header.
            if let Some(q) = quote.map(|q| q.replace(['\n', '\r'], " ")).filter(|q| !q.trim().is_empty()) {
                header.push_str(&format!("quote: {}\n", q.trim()));
            }
        }
        for (n, f) in files.iter().enumerate() {
            let size = std::fs::metadata(f).map_err(|e| format!("{}: {e}", f.display()))?.len();
            if size > MAX_UPLOAD {
                return Err(format!("{} is larger than {} MB", f.display(), MAX_UPLOAD >> 20));
            }
            let file_name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let entry = format!("f-{ms:013}-{n}-{}", clean(&file_name));
            let path = f.to_string_lossy().into_owned();
            let blob = self.chat_git(&["hash-object", "-w", "--", &path])?;
            header.push_str(&format!("attach: {entry}\n"));
            entries.push((entry, blob));
        }
        let content = format!("{header}\n{body}");
        let blob = text(git(&self.root, &["hash-object", "-w", "--stdin"], &[], Some(content.as_bytes()))?);
        entries.push((format!("m-{ms:013}-{}.txt", unique()), blob));

        let parent = self.chat_ref(LOCAL);
        let commit = self.chat_commit(&entries, parent.as_deref(), None)?;
        let mut args = vec!["update-ref", LOCAL, commit.as_str()];
        if let Some(p) = &parent {
            args.push(p);
        }
        self.chat_git(&args).map(|_| ())
    }

    /// The newest `limit` messages on the local chat branch, oldest first.
    pub fn chat_messages(&self, limit: usize) -> Result<Vec<ChatMessage>> {
        if self.chat_ref(LOCAL).is_none() {
            return Ok(Vec::new());
        }
        let listing = self.chat_git(&["ls-tree", "--name-only", LOCAL])?;
        let mut names: Vec<&str> = listing.lines().filter(|n| n.starts_with("m-")).collect();
        names.sort_unstable();
        let names = &names[names.len().saturating_sub(limit)..];
        let request: String = names.iter().map(|n| format!("{LOCAL}:{n}\n")).collect();
        let out = git(&self.root, &["cat-file", "--batch"], &[], Some(request.as_bytes()))?;
        Ok(parse_batch(&out, names))
    }

    /// Save an uploaded file to `dest`.
    pub fn chat_export(&self, entry: &str, dest: &Path) -> Result<()> {
        let bytes = git(&self.root, &["cat-file", "blob", &format!("{LOCAL}:{entry}")], &[], None)?;
        std::fs::write(dest, bytes).map_err(|e| e.to_string())
    }

    /// Exchange chat with `origin`: fetch, merge, push. Returns a short summary.
    pub fn chat_sync(&self) -> Result<String> {
        self.chat_git(&["remote", "get-url", "origin"]).map_err(|_| "This repository has no `origin` remote".to_string())?;
        for attempt in 0..2 {
            // A missing remote branch just means nobody has pushed chat yet.
            let spec = format!("+{LOCAL}:{REMOTE}");
            if let Err(e) = self.chat_git(&["fetch", "origin", &spec]) {
                if !e.contains("couldn't find remote ref") {
                    return Err(e);
                }
            }
            self.chat_merge()?;
            let Some(local) = self.chat_ref(LOCAL) else {
                return Ok("No messages yet".into());
            };
            if self.chat_ref(REMOTE).as_deref() == Some(local.as_str()) {
                return Ok("Chat is up to date".into());
            }
            match self.chat_git(&["push", "origin", &format!("{LOCAL}:{LOCAL}")]) {
                Ok(_) => return Ok("Chat synced".into()),
                // Someone pushed in between: fetch and merge once more.
                Err(e) if attempt == 0 && (e.contains("rejected") || e.contains("fetch first")) => continue,
                Err(e) => return Err(e),
            }
        }
        Err("Could not push chat".into())
    }

    /// Bring the local chat branch up to the union of local and remote.
    fn chat_merge(&self) -> Result<()> {
        let (local, remote) = (self.chat_ref(LOCAL), self.chat_ref(REMOTE));
        let (local, remote) = match (local, remote) {
            (_, None) => return Ok(()),
            (None, Some(r)) => return self.chat_git(&["update-ref", LOCAL, &r]).map(|_| ()),
            (Some(l), Some(r)) => (l, r),
        };
        if local == remote || self.chat_git(&["merge-base", "--is-ancestor", &remote, &local]).is_ok() {
            return Ok(());
        }
        if self.chat_git(&["merge-base", "--is-ancestor", &local, &remote]).is_ok() {
            return self.chat_git(&["update-ref", LOCAL, &remote, &local]).map(|_| ());
        }
        // Diverged: keep everything from both sides.
        let have = self.chat_git(&["ls-tree", "--name-only", &local])?;
        let have: std::collections::HashSet<&str> = have.lines().collect();
        let theirs = self.chat_git(&["ls-tree", &remote])?;
        let entries: Vec<(String, String)> = theirs
            .lines()
            .filter_map(|l| {
                let (meta, name) = l.split_once('\t')?;
                let blob = meta.split_whitespace().nth(2)?;
                (!have.contains(name)).then(|| (name.to_string(), blob.to_string()))
            })
            .collect();
        let commit = self.chat_commit(&entries, Some(&local), Some(&remote))?;
        self.chat_git(&["update-ref", LOCAL, &commit, &local]).map(|_| ())
    }
}

/// Read the output of `git cat-file --batch` for `names`, in order.
fn parse_batch(mut out: &[u8], names: &[&str]) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for name in names {
        let Some(nl) = out.iter().position(|&b| b == b'\n') else { break };
        let head = String::from_utf8_lossy(&out[..nl]).into_owned();
        out = &out[nl + 1..];
        let mut parts = head.split_whitespace();
        let size: usize = match (parts.next(), parts.next(), parts.next().and_then(|s| s.parse().ok())) {
            (Some(_), Some("blob"), Some(size)) => size,
            _ => continue, // "<name> missing"
        };
        if out.len() < size {
            break;
        }
        let body = String::from_utf8_lossy(&out[..size]).into_owned();
        out = out.get(size + 1..).unwrap_or(&[]);
        messages.push(parse_message(name, &body));
    }
    messages
}

pub fn parse_message(id: &str, content: &str) -> ChatMessage {
    let (head, body) = content.split_once("\n\n").unwrap_or((content, ""));
    let mut m = ChatMessage {
        id: id.to_string(),
        author: "someone".into(),
        login: None,
        time_ms: 0,
        body: body.to_string(),
        attachments: Vec::new(),
        reply_to: None,
        quote: None,
    };
    for line in head.lines() {
        match line.split_once(": ") {
            Some(("author", v)) => m.author = v.to_string(),
            Some(("login", v)) if super::is_github_login(v) => m.login = Some(v.to_string()),
            Some(("time", v)) => m.time_ms = v.parse().unwrap_or(0),
            Some(("reply", v)) => m.reply_to = Some(v.to_string()),
            Some(("quote", v)) => m.quote = Some(v.to_string()),
            Some(("attach", v)) => {
                let name = v.splitn(4, '-').nth(3).unwrap_or(v).to_string();
                m.attachments.push(Attachment { entry: v.to_string(), name });
            }
            _ => {}
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn clone_of(remote: &Path, dir: &Path, who: &str) -> Repo {
        let out = Command::new("git").arg("clone").arg(remote).arg(dir).output().unwrap();
        assert!(out.status.success());
        sh(dir, &["config", "user.name", who]);
        sh(dir, &["config", "user.email", &format!("{who}@example.com")]);
        Repo::open(dir).unwrap()
    }

    #[test]
    fn two_people_chat_through_a_remote() {
        let base = std::env::temp_dir().join(format!("hythe-chat-{}-{}", std::process::id(), unique()));
        let remote = base.join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "--bare", "-q"]);
        let (a, b) = (clone_of(&remote, &base.join("a"), "ann"), clone_of(&remote, &base.join("b"), "bob"));

        assert!(a.chat_messages(50).unwrap().is_empty());
        assert_eq!(a.chat_sync().unwrap(), "No messages yet");

        let pic = base.join("pic.png");
        std::fs::write(&pic, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
        a.chat_post("hello from ann", &[pic.clone()], None, None).unwrap();
        b.chat_post("bob here\n\nwith two paragraphs", &[], None, None).unwrap();
        a.chat_sync().unwrap();
        // Bob's history diverges from Ann's: his sync must merge, then push.
        b.chat_sync().unwrap();
        a.chat_sync().unwrap();

        for repo in [&a, &b] {
            let msgs = repo.chat_messages(50).unwrap();
            assert_eq!(msgs.len(), 2, "{msgs:?}");
            assert!(msgs.iter().any(|m| m.author == "ann" && m.body == "hello from ann" && m.attachments.len() == 1));
            assert!(msgs.iter().any(|m| m.author == "bob" && m.body.contains("two paragraphs")));
        }
        let att = a.chat_messages(50).unwrap().into_iter().find_map(|m| m.attachments.into_iter().next()).unwrap();
        assert_eq!(att.name, "pic.png");
        assert!(att.is_image());
        let saved = base.join("saved.png");
        b.chat_export(&att.entry, &saved).unwrap();
        assert_eq!(std::fs::read(saved).unwrap(), [0x89, b'P', b'N', b'G', 1, 2, 3]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn parses_headers_and_body() {
        let m = parse_message(
            "m-1",
            "author: Ann Lee\nlogin: ann-lee\ntime: 42\nreply: m-0000000000041-ab12.txt\nquote: line one\nattach: f-0000000000042-0-a_b.zip\n\nline one\n\nline two",
        );
        assert_eq!((m.author.as_str(), m.login.as_deref(), m.time_ms), ("Ann Lee", Some("ann-lee"), 42));
        assert_eq!(m.body, "line one\n\nline two");
        assert_eq!(m.attachments[0].name, "a_b.zip");
        assert_eq!(m.reply_to.as_deref(), Some("m-0000000000041-ab12.txt"));
        assert_eq!(m.quote.as_deref(), Some("line one"));
    }
}
