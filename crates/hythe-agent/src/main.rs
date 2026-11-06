//! `hythe-agent`: how AI agents inside Hythe talk to each other.
//!
//! Hythe starts each agent with these environment variables:
//!
//!   HYTHE_AGENT_NAME   who you are (e.g. `fable`, `opus`)
//!   HYTHE_THREAD       the thread you are working in
//!
//! and runs it in the repository, so no other setup is needed. Messages, notes
//! and read receipts are stored on the repository's `hythe-threads` branch and
//! shared with teammates when Hythe syncs.

use std::io::Read;
use std::process::ExitCode;

use hythe_git::threads::{clean_name, Kind, Post};
use hythe_git::Repo;

const HELP: &str = "\
hythe-agent: talk to other agents and people in this repository

  hythe-agent who                          your name and the current thread
  hythe-agent agents                       who else is taking part
  hythe-agent say <to> <message...>        message an agent or person (to: a name, or a,b,c)
  hythe-agent inbox                        unread messages addressed to you
  hythe-agent read [N]                     the last N messages in this thread (default 20)
  hythe-agent note <topic> <text...>       leave or update a note others will read (like a README)
  hythe-agent notes [topic]                list notes, or show one in full

Use `-` as the message or text to read it from standard input.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hythe-agent: {e}");
            ExitCode::from(1)
        }
    }
}

fn me() -> Result<String, String> {
    let n = std::env::var("HYTHE_AGENT_NAME").map(|n| clean_name(&n)).unwrap_or_default();
    if n.is_empty() {
        Err("HYTHE_AGENT_NAME is not set; this command is meant to be run by an agent started from Hythe".into())
    } else {
        Ok(n)
    }
}

fn thread() -> Result<String, String> {
    std::env::var("HYTHE_THREAD").ok().filter(|t| !t.is_empty()).ok_or_else(|| "HYTHE_THREAD is not set".into())
}

fn repo() -> Result<Repo, String> {
    let dir = std::env::current_dir().map_err(|e| e.to_string())?;
    Repo::open(&dir).ok_or_else(|| "not inside a git repository".to_string())
}

fn body(words: &[String]) -> Result<String, String> {
    let text = if words == ["-"] {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
        s
    } else {
        words.join(" ")
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        Err("empty message".into())
    } else {
        Ok(text)
    }
}

fn when(ms: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let s = ((now - ms) / 1000).max(0);
    match s {
        0..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

pub fn run(args: &[String]) -> Result<String, String> {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    match cmd {
        "help" | "--help" | "-h" => Ok(HELP.into()),
        "who" => Ok(format!("you are {}\nthread: {}", me()?, thread().unwrap_or_else(|e| e))),
        "agents" => {
            let repo = repo()?;
            let mut names: Vec<String> = repo
                .thread_events()?
                .into_iter()
                .filter(|e| e.kind == Kind::Message)
                .map(|e| e.agent.unwrap_or(e.author))
                .collect();
            if let Ok(list) = std::env::var("HYTHE_AGENTS") {
                names.extend(list.split(',').map(clean_name));
            }
            names.sort();
            names.dedup();
            names.retain(|n| !n.is_empty());
            Ok(if names.is_empty() { "nobody else yet".into() } else { names.join("\n") })
        }
        "say" => {
            let (to, words) = rest.split_first().ok_or("usage: hythe-agent say <to> <message...>")?;
            let recipients: Vec<String> = to.split(',').map(clean_name).filter(|t| !t.is_empty()).collect();
            if recipients.is_empty() {
                return Err("say who? e.g. hythe-agent say opus \"please review this\"".into());
            }
            let text = body(words)?;
            let repo = repo()?;
            let me = me()?;
            repo.thread_post(&Post { thread: &thread()?, agent: Some(&me), to: &recipients, body: &text, ..Default::default() })?;
            Ok(format!("sent to {}", recipients.join(", ")))
        }
        "inbox" => {
            let repo = repo()?;
            let me = me()?;
            let msgs = repo.thread_inbox(&me)?;
            if msgs.is_empty() {
                return Ok("no new messages".into());
            }
            let mut out = String::new();
            let mut by_thread: std::collections::BTreeMap<String, Vec<String>> = Default::default();
            for m in &msgs {
                out.push_str(&format!("[{}] {} ({}):\n{}\n\n", m.thread, m.author, when(m.time_ms), m.body.trim()));
                by_thread.entry(m.thread.clone()).or_default().push(m.id.clone());
            }
            for (t, ids) in by_thread {
                repo.thread_ack(&t, &me, &ids)?;
            }
            Ok(out.trim_end().to_string())
        }
        "read" => {
            let n: usize = rest.first().and_then(|s| s.parse().ok()).unwrap_or(20);
            let msgs = repo()?.thread_messages(&thread()?)?;
            let start = msgs.len().saturating_sub(n);
            let out: Vec<String> = msgs[start..]
                .iter()
                .map(|m| {
                    let to = if m.to.is_empty() { String::new() } else { format!(" -> {}", m.to.join(",")) };
                    format!("{}{} ({}): {}", m.author, to, when(m.time_ms), m.body.trim())
                })
                .collect();
            Ok(if out.is_empty() { "the thread is empty".into() } else { out.join("\n\n") })
        }
        "note" => {
            let (topic, words) = rest.split_first().ok_or("usage: hythe-agent note <topic> <text...>")?;
            let text = body(words)?;
            let me = me()?;
            repo()?.thread_post(&Post { thread: &thread()?, kind: Some(Kind::Note), agent: Some(&me), topic: Some(topic), body: &text, ..Default::default() })?;
            Ok(format!("saved note `{}`", clean_name(topic)))
        }
        "notes" => {
            let notes = repo()?.thread_notes()?;
            match rest.first().map(|t| clean_name(t)) {
                Some(topic) => notes
                    .into_iter()
                    .find(|n| n.topic == topic)
                    .map(|n| format!("# {} (by {}, {})\n\n{}", n.topic, n.author, when(n.time_ms), n.body.trim()))
                    .ok_or_else(|| format!("no note called `{topic}`")),
                None if notes.is_empty() => Ok("no notes yet".into()),
                None => Ok(notes
                    .iter()
                    .map(|n| format!("{} (by {}, {}): {}", n.topic, n.author, when(n.time_ms), n.body.lines().next().unwrap_or("").trim()))
                    .collect::<Vec<_>>()
                    .join("\n")),
            }
        }
        other => Err(format!("unknown command `{other}`; try `hythe-agent help`")),
    }
}
