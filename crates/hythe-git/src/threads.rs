//! Threads that people and AI agents share, kept in git.
//!
//! Like chat, this lives on an orphan branch (`hythe-threads`) that is never
//! checked out. Everything is an immutable *event*, one file each, so two
//! machines' histories merge by union:
//!
//! - `message`: someone (a person or an agent) says something, optionally
//!   `to` named agents/people, optionally a comment on a quoted phrase.
//! - `note`: a durable README-style note on a `topic`; the newest per topic wins.
//! - `rename`, `archive`: thread bookkeeping.
//! - `delete`: hides a message (`target`) or a whole thread (no target).
//! - `ack`: an addressee marks a message as read.
//!
//! Agents use this through the `hythe-agent` command, so any CLI model that can
//! run a shell command can talk to the others.

use std::collections::{HashMap, HashSet};

use super::chat::{git, now_ms, text, unique, Store};
use super::{Repo, Result};

const THREADS: Store = Store {
    local: "refs/heads/hythe-threads",
    remote: "refs/remotes/origin/hythe-threads",
    index: "hythe-threads-index",
    nothing: "No threads yet",
    synced: "Threads synced",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Message,
    Note,
    Rename,
    Archive,
    Delete,
    Ack,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Message => "message",
            Kind::Note => "note",
            Kind::Rename => "rename",
            Kind::Archive => "archive",
            Kind::Delete => "delete",
            Kind::Ack => "ack",
        }
    }

    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "message" => Kind::Message,
            "note" => Kind::Note,
            "rename" => Kind::Rename,
            "archive" => Kind::Archive,
            "delete" => Kind::Delete,
            "ack" => Kind::Ack,
            _ => return None,
        })
    }
}

/// One thing that happened in a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// File name; sorts by time.
    pub id: String,
    pub thread: String,
    pub kind: Kind,
    /// Person's name, or the agent's.
    pub author: String,
    /// Set when an AI agent wrote it (its name, e.g. `fable`).
    pub agent: Option<String>,
    pub login: Option<String>,
    pub time_ms: i64,
    /// Who it is for (agent or person names, lowercase).
    pub to: Vec<String>,
    pub reply_to: Option<String>,
    pub quote: Option<String>,
    /// For `delete` and `ack`: the event it is about.
    pub target: Option<String>,
    /// For `note`: the topic.
    pub topic: Option<String>,
    pub body: String,
}

/// What to post; the store fills in id, time and who the git user is.
#[derive(Debug, Clone, Default)]
pub struct Post<'a> {
    pub thread: &'a str,
    pub kind: Option<Kind>,
    /// Post as this agent instead of the git user.
    pub agent: Option<&'a str>,
    pub to: &'a [String],
    pub reply_to: Option<&'a str>,
    pub quote: Option<&'a str>,
    pub target: Option<&'a str>,
    pub topic: Option<&'a str>,
    pub body: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfo {
    pub id: String,
    pub title: String,
    pub last_ms: i64,
    pub messages: usize,
    pub archived: bool,
    /// Names that have spoken in it (people and agents).
    pub participants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub topic: String,
    pub body: String,
    pub author: String,
    pub time_ms: i64,
    pub thread: String,
}

/// A name safe to use in headers and addressing: lowercase letters, digits, `-`, `_`, `.`.
pub fn clean_name(s: &str) -> String {
    s.trim()
        .trim_start_matches('@')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c.to_ascii_lowercase() } else { '-' })
        .take(40)
        .collect()
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ").trim().to_string()
}

fn valid_id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix) && s.len() < 80 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

impl Repo {
    /// Start a thread and return its id.
    pub fn thread_new(&self, title: &str) -> Result<String> {
        let id = format!("t-{:013}-{}", now_ms(), unique());
        self.thread_post(&Post { thread: &id, kind: Some(Kind::Rename), body: title, ..Default::default() })?;
        Ok(id)
    }

    /// Append an event to the local threads branch. Returns the new event's id.
    pub fn thread_post(&self, p: &Post) -> Result<String> {
        if !valid_id(p.thread, "t-") {
            return Err(format!("bad thread id `{}`", p.thread));
        }
        let kind = p.kind.unwrap_or(Kind::Message);
        let (author, login) = self.chat_identity();
        let ms = now_ms();
        let id = format!("e-{ms:013}-{}.txt", unique());
        let agent = p.agent.map(clean_name).filter(|a| !a.is_empty());
        let mut head = format!("kind: {}\nthread: {}\n", kind.name(), p.thread);
        head.push_str(&format!("author: {}\n", agent.clone().unwrap_or_else(|| one_line(&author))));
        if let Some(a) = &agent {
            head.push_str(&format!("agent: {a}\n"));
        } else if let Some(l) = &login {
            head.push_str(&format!("login: {l}\n"));
        }
        head.push_str(&format!("time: {ms}\n"));
        let to: Vec<String> = p.to.iter().map(|t| clean_name(t)).filter(|t| !t.is_empty()).collect();
        if !to.is_empty() {
            head.push_str(&format!("to: {}\n", to.join(",")));
        }
        if let Some(r) = p.reply_to.filter(|r| valid_id(r, "e-")) {
            head.push_str(&format!("reply: {r}\n"));
            if let Some(q) = p.quote.map(one_line).filter(|q| !q.is_empty()) {
                head.push_str(&format!("quote: {q}\n"));
            }
        }
        if let Some(t) = p.target.filter(|t| valid_id(t, "e-")) {
            head.push_str(&format!("target: {t}\n"));
        }
        if let Some(t) = p.topic.map(clean_name).filter(|t| !t.is_empty()) {
            head.push_str(&format!("topic: {t}\n"));
        }
        let content = format!("{head}\n{}", p.body);
        let blob = text(git(&self.root, &["hash-object", "-w", "--stdin"], &[], Some(content.as_bytes()))?);
        let parent = self.chat_ref(THREADS.local);
        let commit = self.chat_commit(&THREADS, &[(id.clone(), blob)], parent.as_deref(), None)?;
        let mut args = vec!["update-ref", THREADS.local, commit.as_str()];
        if let Some(p) = &parent {
            args.push(p);
        }
        self.chat_git(&args)?;
        Ok(id)
    }

    /// Every event on the local threads branch, oldest first.
    pub fn thread_events(&self) -> Result<Vec<Event>> {
        if self.chat_ref(THREADS.local).is_none() {
            return Ok(Vec::new());
        }
        let listing = self.chat_git(&["ls-tree", "--name-only", THREADS.local])?;
        let mut names: Vec<&str> = listing.lines().filter(|n| n.starts_with("e-")).collect();
        names.sort_unstable();
        let request: String = names.iter().map(|n| format!("{}:{n}\n", THREADS.local)).collect();
        let out = git(&self.root, &["cat-file", "--batch"], &[], Some(request.as_bytes()))?;
        Ok(parse_events(&out, &names))
    }

    /// Threads that aren't deleted, newest activity first.
    pub fn thread_list(&self) -> Result<Vec<ThreadInfo>> {
        Ok(summarize(&self.thread_events()?))
    }

    /// A thread's visible messages (deleted ones removed), oldest first.
    pub fn thread_messages(&self, thread: &str) -> Result<Vec<Event>> {
        Ok(visible_messages(&self.thread_events()?, thread))
    }

    /// Unread messages addressed to `who`, oldest first.
    pub fn thread_inbox(&self, who: &str) -> Result<Vec<Event>> {
        Ok(inbox(&self.thread_events()?, &clean_name(who)))
    }

    /// The newest note per topic.
    pub fn thread_notes(&self) -> Result<Vec<Note>> {
        Ok(notes(&self.thread_events()?))
    }

    /// Hide a message.
    pub fn thread_delete_message(&self, thread: &str, message: &str) -> Result<()> {
        self.thread_post(&Post { thread, kind: Some(Kind::Delete), target: Some(message), ..Default::default() }).map(|_| ())
    }

    /// Hide a whole thread.
    pub fn thread_delete(&self, thread: &str) -> Result<()> {
        self.thread_post(&Post { thread, kind: Some(Kind::Delete), ..Default::default() }).map(|_| ())
    }

    /// Mark messages as read by `who`.
    pub fn thread_ack(&self, thread: &str, who: &str, ids: &[String]) -> Result<()> {
        for id in ids {
            self.thread_post(&Post { thread, kind: Some(Kind::Ack), agent: Some(who), target: Some(id), ..Default::default() })?;
        }
        Ok(())
    }

    /// Exchange threads with `origin`.
    pub fn thread_sync(&self) -> Result<String> {
        self.store_sync(&THREADS)
    }
}

fn parse_events(mut out: &[u8], names: &[&str]) -> Vec<Event> {
    let mut events = Vec::new();
    for name in names {
        let Some(nl) = out.iter().position(|&b| b == b'\n') else { break };
        let head = String::from_utf8_lossy(&out[..nl]).into_owned();
        out = &out[nl + 1..];
        let mut parts = head.split_whitespace();
        let size: usize = match (parts.next(), parts.next(), parts.next().and_then(|s| s.parse().ok())) {
            (Some(_), Some("blob"), Some(size)) => size,
            _ => continue,
        };
        if out.len() < size {
            break;
        }
        let body = String::from_utf8_lossy(&out[..size]).into_owned();
        out = out.get(size + 1..).unwrap_or(&[]);
        if let Some(e) = parse_event(name, &body) {
            events.push(e);
        }
    }
    events
}

pub fn parse_event(id: &str, content: &str) -> Option<Event> {
    let (head, body) = content.split_once("\n\n").unwrap_or((content, ""));
    let mut e = Event {
        id: id.to_string(),
        thread: String::new(),
        kind: Kind::Message,
        author: "someone".into(),
        agent: None,
        login: None,
        time_ms: 0,
        to: Vec::new(),
        reply_to: None,
        quote: None,
        target: None,
        topic: None,
        body: body.to_string(),
    };
    for line in head.lines() {
        match line.split_once(": ") {
            Some(("kind", v)) => e.kind = Kind::parse(v)?,
            Some(("thread", v)) if valid_id(v, "t-") => e.thread = v.to_string(),
            Some(("author", v)) => e.author = v.to_string(),
            Some(("agent", v)) => e.agent = Some(clean_name(v)),
            Some(("login", v)) if super::is_github_login(v) => e.login = Some(v.to_string()),
            Some(("time", v)) => e.time_ms = v.parse().unwrap_or(0),
            Some(("to", v)) => e.to = v.split(',').map(clean_name).filter(|t| !t.is_empty()).collect(),
            Some(("reply", v)) if valid_id(v, "e-") => e.reply_to = Some(v.to_string()),
            Some(("quote", v)) => e.quote = Some(v.to_string()),
            Some(("target", v)) if valid_id(v, "e-") => e.target = Some(v.to_string()),
            Some(("topic", v)) => e.topic = Some(clean_name(v)),
            _ => {}
        }
    }
    (!e.thread.is_empty()).then_some(e)
}

fn deleted_ids(events: &[Event]) -> (HashSet<&str>, HashSet<&str>) {
    let mut messages = HashSet::new();
    let mut threads = HashSet::new();
    for e in events.iter().filter(|e| e.kind == Kind::Delete) {
        match &e.target {
            Some(t) => {
                messages.insert(t.as_str());
            }
            None => {
                threads.insert(e.thread.as_str());
            }
        }
    }
    (messages, threads)
}

pub fn visible_messages(events: &[Event], thread: &str) -> Vec<Event> {
    let (gone, _) = deleted_ids(events);
    events
        .iter()
        .filter(|e| e.thread == thread && e.kind == Kind::Message && !gone.contains(e.id.as_str()))
        .cloned()
        .collect()
}

pub fn summarize(events: &[Event]) -> Vec<ThreadInfo> {
    let (gone, dead) = deleted_ids(events);
    let mut map: HashMap<&str, ThreadInfo> = HashMap::new();
    for e in events {
        if dead.contains(e.thread.as_str()) {
            continue;
        }
        let info = map.entry(e.thread.as_str()).or_insert_with(|| ThreadInfo {
            id: e.thread.clone(),
            title: String::new(),
            last_ms: 0,
            messages: 0,
            archived: false,
            participants: Vec::new(),
        });
        info.last_ms = info.last_ms.max(e.time_ms);
        match e.kind {
            Kind::Rename => info.title = one_line(&e.body),
            Kind::Archive => info.archived = e.body.trim() != "restore",
            Kind::Message if !gone.contains(e.id.as_str()) => {
                info.messages += 1;
                if !info.participants.contains(&e.author) {
                    info.participants.push(e.author.clone());
                }
            }
            _ => {}
        }
    }
    let mut list: Vec<ThreadInfo> = map.into_values().collect();
    for t in &mut list {
        if t.title.is_empty() {
            t.title = "Untitled thread".into();
        }
    }
    list.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.id.cmp(&b.id)));
    list
}

pub fn inbox(events: &[Event], who: &str) -> Vec<Event> {
    let (gone, dead) = deleted_ids(events);
    let read: HashSet<&str> = events
        .iter()
        .filter(|e| e.kind == Kind::Ack && e.agent.as_deref() == Some(who))
        .filter_map(|e| e.target.as_deref())
        .collect();
    events
        .iter()
        .filter(|e| e.kind == Kind::Message && e.to.iter().any(|t| t == who))
        .filter(|e| e.agent.as_deref() != Some(who) && e.author != who)
        .filter(|e| !gone.contains(e.id.as_str()) && !dead.contains(e.thread.as_str()) && !read.contains(e.id.as_str()))
        .cloned()
        .collect()
}

pub fn notes(events: &[Event]) -> Vec<Note> {
    let mut latest: HashMap<&str, &Event> = HashMap::new();
    for e in events.iter().filter(|e| e.kind == Kind::Note) {
        if let Some(topic) = &e.topic {
            // Events are oldest first, so later ones replace earlier ones.
            latest.insert(topic.as_str(), e);
        }
    }
    let mut out: Vec<Note> = latest
        .into_iter()
        .map(|(topic, e)| Note { topic: topic.to_string(), body: e.body.clone(), author: e.author.clone(), time_ms: e.time_ms, thread: e.thread.clone() })
        .filter(|n| !n.body.trim().is_empty())
        .collect();
    out.sort_by(|a, b| a.topic.cmp(&b.topic));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

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

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("hythe-threads-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn pair(tag: &str) -> (std::path::PathBuf, Repo, Repo) {
        let base = tmp(tag);
        std::fs::create_dir_all(&base).unwrap();
        let remote = base.join("remote.git");
        sh(&base, &["init", "-q", "--bare", remote.to_str().unwrap()]);
        let a = clone_of(&remote, &base.join("a"), "Ann");
        let b = clone_of(&remote, &base.join("b"), "Bob");
        (base, a, b)
    }

    #[test]
    fn agents_message_each_other_and_leave_notes() {
        let (base, a, _b) = pair("agents");
        let t = a.thread_new("Refactor the cache").unwrap();
        let q = a
            .thread_post(&Post { thread: &t, agent: Some("Fable"), to: &["Opus".into()], body: "Can you review sum_tree.rs?", ..Default::default() })
            .unwrap();
        // Only the addressee has it in their inbox, and the sender doesn't.
        assert_eq!(a.thread_inbox("opus").unwrap().len(), 1);
        assert!(a.thread_inbox("fable").unwrap().is_empty());
        assert!(a.thread_inbox("sonnet").unwrap().is_empty());
        a.thread_ack(&t, "opus", &[q]).unwrap();
        assert!(a.thread_inbox("opus").unwrap().is_empty(), "acked messages leave the inbox");

        // Notes: newest per topic wins.
        a.thread_post(&Post { thread: &t, kind: Some(Kind::Note), agent: Some("fable"), topic: Some("Cache Design"), body: "v1", ..Default::default() }).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        a.thread_post(&Post { thread: &t, kind: Some(Kind::Note), agent: Some("opus"), topic: Some("cache design"), body: "v2: use 16 byte nodes", ..Default::default() }).unwrap();
        let notes = a.thread_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].topic, "cache-design");
        assert_eq!(notes[0].body, "v2: use 16 byte nodes");
        assert_eq!(notes[0].author, "opus");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn delete_hides_messages_and_threads_and_syncs_between_people() {
        let (base, a, b) = pair("sync");
        let t = a.thread_new("Plan").unwrap();
        let m1 = a.thread_post(&Post { thread: &t, body: "keep", ..Default::default() }).unwrap();
        let m2 = a.thread_post(&Post { thread: &t, body: "oops", ..Default::default() }).unwrap();
        a.thread_sync().unwrap();
        b.thread_sync().unwrap();
        // Both people write concurrently, then sync: everyone ends with the union.
        b.thread_post(&Post { thread: &t, body: "from bob", ..Default::default() }).unwrap();
        a.thread_post(&Post { thread: &t, body: "from ann", ..Default::default() }).unwrap();
        a.thread_delete_message(&t, &m2).unwrap();
        a.thread_sync().unwrap();
        b.thread_sync().unwrap();
        a.thread_sync().unwrap();
        for repo in [&a, &b] {
            let bodies: Vec<String> = repo.thread_messages(&t).unwrap().into_iter().map(|e| e.body).collect();
            assert_eq!(bodies.len(), 3, "{bodies:?}");
            assert!(bodies.contains(&"keep".to_string()) && bodies.contains(&"from bob".to_string()) && bodies.contains(&"from ann".to_string()));
            assert!(!bodies.contains(&"oops".to_string()));
        }
        let _ = m1;
        let list = b.thread_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Plan");
        assert_eq!(list[0].messages, 3);
        b.thread_delete(&t).unwrap();
        assert!(b.thread_list().unwrap().is_empty());
        b.thread_sync().unwrap();
        a.thread_sync().unwrap();
        assert!(a.thread_list().unwrap().is_empty(), "deleting a thread syncs to others");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn names_and_ids_are_sanitised() {
        assert_eq!(clean_name("@Claude Opus!"), "claude-opus-");
        let e = parse_event("e-1.txt", "kind: message\nthread: t-1\nto: A,B C\n\nhi").unwrap();
        assert_eq!(e.to, vec!["a", "b-c"]);
        assert!(parse_event("e-1.txt", "kind: message\nthread: ../x\n\nhi").is_none());
        assert!(parse_event("e-1.txt", "kind: bogus\nthread: t-1\n\nhi").is_none());
    }
}
