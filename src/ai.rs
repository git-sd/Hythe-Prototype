//! Threads with AI agents, drawn as a conversation like Zed's agent panel.
//!
//! Each message runs an agent CLI in the project folder (`claude -p`, the prompt
//! on stdin, JSON out) and resumes that agent's session for follow-ups. File
//! edits are accepted, so they land in the working tree and show up in the
//! Changes tab. `HYTHE_AGENT_CMD` names another program to run instead, for tests.
//!
//! Agents are not alone: every thread is stored in git (`hythe-threads`), and each
//! agent is told how to message the others and leave notes with the `hythe-agent`
//! command. Address an agent with `@name` (`@opus`, `@fable`, ...); messages agents
//! send each other are delivered by running the addressed agent, up to [`MAX_HOPS`]
//! agent-to-agent steps before it waits for you.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding, Task, Theme};

use crate::icons::{icon, Icon};
use crate::{theme, App, Message};

/// How many times agents may hand work to each other before waiting for a person.
pub const MAX_HOPS: u32 = 6;

/// An agent that can take part in a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDef {
    /// Lowercase handle used in `@mentions` and `hythe-agent say`.
    pub name: String,
    pub label: String,
    /// Passed as `--model` to Claude Code.
    pub model: Option<String>,
    /// Another CLI: prompt on stdin, answer on stdout.
    pub plain: Option<String>,
}

fn claude(name: &str, label: &str, model: Option<&str>) -> AgentDef {
    AgentDef { name: name.into(), label: label.into(), model: model.map(str::to_string), plain: None }
}

/// Everyone who can be mentioned: Claude Code with different models, plus other
/// CLIs found on this machine or named in `HYTHE_EXTRA_AGENTS` (`name=command,name2=command`).
pub fn agents() -> Vec<AgentDef> {
    let mut list = vec![
        claude("claude", "Claude", None),
        claude("fable", "Fable", Some("claude-fable-5-1")),
        claude("opus", "Opus", Some("claude-opus-5-5")),
        claude("sonnet", "Sonnet", Some("claude-sonnet-5-5")),
        claude("haiku", "Haiku", Some("claude-haiku-5-5")),
    ];
    for (name, label, cmd) in [("codex", "Codex", "codex exec -"), ("gemini", "Gemini", "gemini")] {
        let installed = hythe_term::known_agents().iter().any(|a| a.installed && a.name.to_lowercase().starts_with(name));
        if installed {
            list.push(AgentDef { name: name.into(), label: label.into(), model: None, plain: Some(cmd.into()) });
        }
    }
    if let Ok(extra) = std::env::var("HYTHE_EXTRA_AGENTS") {
        for item in extra.split(',') {
            if let Some((n, cmd)) = item.split_once('=') {
                let name = hythe_git::threads::clean_name(n);
                if !name.is_empty() && !cmd.trim().is_empty() && !list.iter().any(|a| a.name == name) {
                    list.push(AgentDef { label: n.trim().to_string(), name, model: None, plain: Some(cmd.trim().to_string()) });
                }
            }
        }
    }
    list
}

/// Who a message is for: the first `@name` that is a known agent, else Claude.
pub fn route(prompt: &str, known: &[AgentDef]) -> String {
    for word in prompt.split_whitespace() {
        if let Some(n) = word.strip_prefix('@') {
            let n = hythe_git::threads::clean_name(n.trim_end_matches(|c: char| !c.is_ascii_alphanumeric()));
            if known.iter().any(|a| a.name == n) {
                return n;
            }
        }
    }
    "claude".into()
}

/// What an agent is told about the others, once, when it joins a thread.
pub fn preamble(me: &AgentDef, thread: &str, known: &[AgentDef]) -> String {
    let others: Vec<String> = known.iter().filter(|a| a.name != me.name).map(|a| format!("@{}", a.name)).collect();
    format!(
        "You are `{name}`, an AI agent working inside Hythe, a code editor, on thread {thread}. \
People and other AI agents ({others}) share this repository and this thread.\n\
\n\
You can work with them using the `hythe-agent` command in your shell:\n\
  hythe-agent notes                       read notes other agents left (do this before starting)\n\
  hythe-agent note <topic> <text>         leave a durable note for the next agent, like a README (decisions, gotchas, status)\n\
  hythe-agent say <agent> <message>       ask another agent for something; it will be started to read it\n\
  hythe-agent inbox                       see messages sent to you\n\
  hythe-agent read                        recent messages in this thread\n\
\n\
Message another agent only when you genuinely need its help or want it to review your work; \
be specific, say what you changed and which files. Agents hand work to each other at most {hops} times \
before waiting for a person, so do not start long back-and-forth. Leave a note when you finish something \
others will build on. Your final answer is shown to the person.\n\n",
        name = me.name,
        others = if others.is_empty() { "none right now".to_string() } else { others.join(", ") },
        hops = MAX_HOPS,
    )
}

fn line(who: &str, to: &[String], text: &str) -> String {
    let to = if to.is_empty() { String::new() } else { format!(" -> {}", to.join(",")) };
    format!("{who}{to}: {}", text.trim())
}

pub struct AiThread {
    /// The project the thread belongs to.
    pub root: PathBuf,
    pub title: String,
    /// Id in the shared thread store, once something was sent.
    pub id: Option<String>,
    /// Each agent's own session id, once it has replied.
    pub sessions: HashMap<String, String>,
    pub turns: Vec<Turn>,
    /// The agent working right now.
    pub running: Option<String>,
    pub input: String,
    /// Agent-to-agent steps since a person last wrote.
    pub hops: u32,
    /// Store events already shown.
    pub seen: HashSet<String>,
}

impl AiThread {
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }
}

pub struct Turn {
    /// Who wrote it: `None` is you; otherwise an agent's or another person's name.
    pub who: Option<String>,
    /// Who it was addressed to.
    pub to: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum AiMsg {
    New,
    Select(usize),
    Input(String),
    Send,
    /// Remove a thread for everyone.
    Delete(usize),
    /// An agent finished a step in thread `n`.
    Done(usize, Box<Done>),
    /// Threads read from the repository (project folder, events, your name).
    Loaded(PathBuf, Result<Vec<hythe_git::threads::Event>, String>, String),
}

/// What happened during one agent step, worked out off the UI thread.
#[derive(Debug, Clone)]
pub struct Done {
    pub agent: String,
    pub thread: Option<String>,
    /// (reply, session id), or why it could not.
    pub result: Result<(String, Option<String>), String>,
    /// Thread messages not shown yet, oldest first (includes the agent's own reply).
    pub events: Vec<hythe_git::threads::Event>,
    /// An agent with messages waiting, and those messages.
    pub next: Option<(String, Vec<hythe_git::threads::Event>)>,
}

/// Everything `run` needs to start an agent.
pub struct Ctx {
    pub agent: AgentDef,
    pub thread: String,
    pub known: Vec<AgentDef>,
    /// Earlier messages, for an agent that has no session yet.
    pub history: String,
}

/// Where `hythe-agent` is: next to this program.
fn tools_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("HYTHE_TOOLS_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Run an agent in `root`, continuing `session` if there is one.
pub fn run(root: &Path, prompt: &str, session: Option<&str>, ctx: &Ctx) -> Result<(String, Option<String>), String> {
    let override_cmd = std::env::var("HYTHE_AGENT_CMD").ok();
    let plain = ctx.agent.plain.clone();
    let program = override_cmd.clone().or_else(|| plain.clone()).unwrap_or_else(|| "claude".into());
    let is_claude = override_cmd.is_none() && plain.is_none();
    let mut args: Vec<String> = Vec::new();
    if is_claude {
        args.extend(["-p", "--output-format", "json", "--permission-mode", "acceptEdits"].map(String::from));
        // Agents may run exactly one shell command: the one that talks to the others.
        args.extend(["--allowedTools".into(), "Bash(hythe-agent:*)".into()]);
        if let Some(m) = &ctx.agent.model {
            args.extend(["--model".into(), m.clone()]);
        }
        // Only ever pass back an id Claude gave us (hex and dashes).
        if let Some(s) = session.filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')) {
            args.extend(["--resume".into(), s.into()]);
        }
    } else if override_cmd.is_some() {
        args.extend(["-p", "--output-format", "json", "--permission-mode", "acceptEdits"].map(String::from));
    }
    // npm installs `claude` as a .cmd shim, which only cmd.exe can start.
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&program);
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        // A plain agent may be "codex exec -": the first word is the program.
        let mut words = program.split_whitespace();
        let mut c = Command::new(words.next().unwrap_or("claude"));
        c.args(words);
        c
    };
    cmd.args(&args).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env("HYTHE_AGENT_NAME", &ctx.agent.name)
        .env("HYTHE_THREAD", &ctx.thread)
        .env("HYTHE_AGENTS", ctx.known.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(","));
    if let Some(dir) = tools_dir() {
        let mut paths = vec![dir];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start {program}: {e}"))?;
    // The prompt goes on stdin, never on the command line.
    let full = if session.is_none() {
        format!("{}{}{prompt}", preamble(&ctx.agent, &ctx.thread, &ctx.known), ctx.history)
    } else {
        prompt.to_string()
    };
    if let Some(mut pipe) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = pipe.write_all(full.as_bytes());
        });
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !is_claude && override_cmd.is_none() {
        // Another CLI: whatever it printed is the answer.
        let text = stdout.trim().to_string();
        return if out.status.success() && !text.is_empty() {
            Ok((text, None))
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.trim().lines().next().unwrap_or("no reply").to_string())
        };
    }
    match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
        Ok(v) => {
            let result = v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string();
            let session = v.get("session_id").and_then(|s| s.as_str()).map(str::to_string);
            if v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false) {
                Err(if result.is_empty() { "the agent reported an error".into() } else { result })
            } else {
                Ok((result, session))
            }
        }
        Err(_) => {
            let err = String::from_utf8_lossy(&out.stderr);
            let why = if err.trim().is_empty() { stdout.trim().to_string() } else { err.trim().to_string() };
            Err(why.lines().next().unwrap_or("no reply").to_string())
        }
    }
}

/// One agent step, start to finish, off the UI thread: record what was said, run
/// the agent, record its answer, then see what everyone else said meanwhile and
/// who now has mail.
#[allow(clippy::too_many_arguments)]
fn step(
    root: PathBuf,
    title: String,
    thread: Option<String>,
    agent: AgentDef,
    session: Option<String>,
    said: Option<String>,
    deliver: Vec<hythe_git::threads::Event>,
    seen: HashSet<String>,
) -> Done {
    use hythe_git::threads::Post;
    let known = agents();
    let repo = hythe_git::Repo::open(&root);
    let mut thread = thread;
    if thread.is_none() {
        thread = repo.as_ref().and_then(|r| r.thread_new(&title).ok());
    }
    let tid = thread.clone().unwrap_or_default();
    // Earlier messages, for an agent that has not been in this thread yet.
    let history = match (&repo, &thread, &session) {
        (Some(r), Some(t), None) => {
            let msgs = r.thread_messages(t).unwrap_or_default();
            let lines: Vec<String> = msgs[msgs.len().saturating_sub(12)..].iter().map(|m| line(&m.author, &m.to, &m.body)).collect();
            if lines.is_empty() {
                String::new()
            } else {
                format!("Earlier in this thread:\n{}\n\n", lines.join("\n"))
            }
        }
        _ => String::new(),
    };
    let mut prompt = String::new();
    let mut mine: HashSet<String> = seen;
    if let (Some(r), Some(text)) = (&repo, &said) {
        if let Ok(id) = r.thread_post(&Post { thread: &tid, to: &[agent.name.clone()], body: text, ..Default::default() }) {
            // It is handed over in the prompt below, so it must not be delivered again.
            let _ = r.thread_ack(&tid, &agent.name, std::slice::from_ref(&id));
            mine.insert(id);
        }
    }
    if let Some(text) = &said {
        prompt = text.clone();
    }
    if !deliver.is_empty() {
        let mut p = String::from("You have new messages from other agents:\n\n");
        for m in &deliver {
            p.push_str(&format!("[{}]: {}\n\n", m.author, m.body.trim()));
        }
        p.push_str("Do the work they ask. To answer them, run `hythe-agent say <name> <message>`; your final answer is shown to the person.");
        if let Some(r) = &repo {
            let ids: Vec<String> = deliver.iter().map(|m| m.id.clone()).collect();
            let _ = r.thread_ack(&tid, &agent.name, &ids);
        }
        prompt = p;
    }
    let ctx = Ctx { agent: agent.clone(), thread: tid.clone(), known: known.clone(), history };
    let result = run(&root, &prompt, session.as_deref(), &ctx);
    if let (Some(r), Ok((reply, _))) = (&repo, &result) {
        if !reply.trim().is_empty() {
            let _ = r.thread_post(&Post { thread: &tid, agent: Some(&agent.name), body: reply, ..Default::default() });
        }
    }
    // What happened in the thread that the window has not shown yet, and who has mail.
    let mut events = Vec::new();
    let mut next = None;
    if let Some(r) = &repo {
        let all = r.thread_events().unwrap_or_default();
        events = hythe_git::threads::visible_messages(&all, &tid).into_iter().filter(|e| !mine.contains(&e.id)).collect();
        for a in &known {
            let mail: Vec<_> = hythe_git::threads::inbox(&all, &a.name).into_iter().filter(|m| m.thread == tid).collect();
            if !mail.is_empty() && a.name != agent.name {
                next = Some((a.name.clone(), mail));
                break;
            }
        }
    }
    Done { agent: agent.name, thread, result, events, next }
}

impl App {
    /// The thread showing in the thread column, if any.
    pub(crate) fn ai_thread(&self) -> Option<&AiThread> {
        let root = self.root.as_ref()?;
        self.ai_active.and_then(|i| self.ai.get(i)).filter(|t| t.root == root.path)
    }

    /// Start an agent step in thread `i`.
    fn ai_step(&mut self, i: usize, agent: String, said: Option<String>, deliver: Vec<hythe_git::threads::Event>) -> Task<Message> {
        let known = agents();
        let Some(def) = known.iter().find(|a| a.name == agent).cloned() else { return Task::none() };
        let Some(t) = self.ai.get_mut(i) else { return Task::none() };
        t.running = Some(agent.clone());
        let (root, title, id, seen) = (t.root.clone(), t.title.clone(), t.id.clone(), t.seen.clone());
        let session = t.sessions.get(&agent).cloned();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || step(root, title, id, def, session, said, deliver, seen))
                    .await
                    .unwrap_or_else(|e| Done { agent, thread: None, result: Err(e.to_string()), events: Vec::new(), next: None })
            },
            move |d| Message::Ai(AiMsg::Done(i, Box::new(d))),
        )
    }

    /// Read the repository's saved threads (once per repository).
    pub(crate) fn ai_load_task(&mut self) -> Task<Message> {
        let Some(repo) = self.repo.clone() else { return Task::none() };
        if self.ai_loaded.as_deref() == Some(repo.root()) {
            return Task::none();
        }
        self.ai_loaded = Some(repo.root().to_path_buf());
        let root = self.root.as_ref().map(|r| r.path.clone()).unwrap_or_else(|| repo.root().to_path_buf());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let me = repo.chat_identity().0;
                    (root, repo.thread_events(), me)
                })
                .await
                .unwrap_or_else(|e| (PathBuf::new(), Err(e.to_string()), String::new()))
            },
            |(root, events, me)| Message::Ai(AiMsg::Loaded(root, events, me)),
        )
    }

    pub(crate) fn ai_update(&mut self, msg: AiMsg) -> Task<Message> {
        match msg {
            AiMsg::New => {
                let Some(root) = self.root.as_ref().map(|r| r.path.clone()) else { return Task::none() };
                self.ai.push(AiThread {
                    root,
                    title: "New thread".into(),
                    id: None,
                    sessions: HashMap::new(),
                    turns: Vec::new(),
                    running: None,
                    input: String::new(),
                    hops: 0,
                    seen: HashSet::new(),
                });
                self.ai_active = Some(self.ai.len() - 1);
                self.show_term = true;
                self.focus_term = false;
            }
            AiMsg::Select(i) => {
                if i < self.ai.len() {
                    self.ai_active = Some(i);
                    self.show_term = true;
                }
            }
            AiMsg::Input(s) => {
                if let Some(t) = self.ai_active.and_then(|i| self.ai.get_mut(i)) {
                    t.input = s;
                }
            }
            AiMsg::Send => {
                let Some(i) = self.ai_active else { return Task::none() };
                let Some(t) = self.ai.get_mut(i) else { return Task::none() };
                let prompt = t.input.trim().to_string();
                if prompt.is_empty() || t.busy() {
                    return Task::none();
                }
                if t.turns.is_empty() {
                    t.title = crate::chat::excerpt(&prompt);
                }
                let agent = route(&prompt, &agents());
                t.turns.push(Turn { who: None, to: vec![agent.clone()], text: prompt.clone() });
                t.input.clear();
                t.hops = 0;
                return self.ai_step(i, agent, Some(prompt), Vec::new());
            }
            AiMsg::Delete(i) => {
                if i >= self.ai.len() {
                    return Task::none();
                }
                let t = self.ai.remove(i);
                self.ai_active = match self.ai_active {
                    Some(a) if a == i => None,
                    Some(a) if a > i => Some(a - 1),
                    other => other,
                };
                self.status = "Thread deleted".into();
                if let (Some(id), Some(repo)) = (t.id, self.repo.clone()) {
                    return Task::perform(
                        async move { tokio::task::spawn_blocking(move || repo.thread_delete(&id)).await.ok() },
                        |_| Message::Noop,
                    );
                }
            }
            AiMsg::Done(i, done) => {
                let done = *done;
                let mut again = Task::none();
                let mut handoff = None;
                if let Some(t) = self.ai.get_mut(i) {
                    t.running = None;
                    if done.thread.is_some() {
                        t.id = done.thread.clone();
                    }
                    // The agent's reply, things agents said to each other, and other voices, in order.
                    let mut shown = 0;
                    for e in &done.events {
                        if t.seen.insert(e.id.clone()) {
                            shown += 1;
                            t.turns.push(Turn { who: Some(e.agent.clone().unwrap_or_else(|| e.author.clone())), to: e.to.clone(), text: e.body.clone() });
                        }
                    }
                    match &done.result {
                        Ok((reply, session)) => {
                            if let Some(s) = session {
                                t.sessions.insert(done.agent.clone(), s.clone());
                            }
                            // Without a saved thread the reply still has to show.
                            if shown == 0 && !reply.trim().is_empty() {
                                t.turns.push(Turn { who: Some(done.agent.clone()), to: Vec::new(), text: reply.clone() });
                            }
                        }
                        Err(e) => t.turns.push(Turn { who: Some(done.agent.clone()), to: Vec::new(), text: format!("Couldn't reach {}: {e}", done.agent) }),
                    }
                    if let Some((agent, mail)) = done.next {
                        if t.hops >= MAX_HOPS {
                            t.turns.push(Turn {
                                who: Some("hythe".into()),
                                to: Vec::new(),
                                text: format!("Agents handed work to each other {MAX_HOPS} times, so I've paused. {agent} has messages waiting; send a message to continue."),
                            });
                        } else {
                            t.hops += 1;
                            handoff = Some((agent, mail));
                        }
                    }
                }
                if let Some((agent, mail)) = handoff {
                    again = self.ai_step(i, agent, None, mail);
                }
                // An agent may have edited files: bring the Changes tab and open files up to date.
                self.refresh_git();
                return again;
            }
            AiMsg::Loaded(root, events, me) => {
                let Ok(all) = events else { return Task::none() };
                for info in hythe_git::threads::summarize(&all).into_iter().filter(|t| !t.archived) {
                    if self.ai.iter().any(|t| t.id.as_deref() == Some(info.id.as_str())) {
                        continue;
                    }
                    let msgs = hythe_git::threads::visible_messages(&all, &info.id);
                    let mut thread = AiThread {
                        root: root.clone(),
                        title: info.title.clone(),
                        id: Some(info.id.clone()),
                        sessions: HashMap::new(),
                        turns: Vec::new(),
                        running: None,
                        input: String::new(),
                        hops: 0,
                        seen: all.iter().filter(|e| e.thread == info.id).map(|e| e.id.clone()).collect(),
                    };
                    for m in msgs {
                        let mine = m.agent.is_none() && m.author == me;
                        let who = if mine { None } else { Some(m.agent.clone().unwrap_or_else(|| m.author.clone())) };
                        thread.turns.push(Turn { who, to: m.to.clone(), text: m.body.clone() });
                    }
                    self.ai.push(thread);
                }
            }
        }
        Task::none()
    }

    /// The conversation: your messages and Claude's, code blocks as cards, and a
    /// frameless box at the bottom to ask for the next change.
    pub(crate) fn ai_view<'a>(&'a self, t: &'a AiThread) -> Element<'a, Message> {
        let mut list = column![].spacing(20).padding(14);
        if t.turns.is_empty() {
            list = list.push(
                text("Ask Claude to explain or change something in this project. Its edits show up in the Changes tab.")
                    .size(13)
                    .color(theme::TEXT_FAINT),
            );
        }
        for turn in &t.turns {
            let face = if turn.who.is_none() { self.my_face() } else { claude_face() };
            let mut body = column![].spacing(8);
            for part in crate::chat::split_code(&turn.text) {
                body = body.push(match part {
                    crate::chat::Part::Text(s) => text(s).size(14).line_height(1.45).into(),
                    crate::chat::Part::Code { info, lines } => crate::chat::code_card(info, lines),
                });
            }
            let body: Element<'a, Message> = match &turn.who {
                Some(who) => {
                    let to = if turn.to.is_empty() { String::new() } else { format!("  ->  {}", turn.to.join(", ")) };
                    column![text(format!("{who}{to}")).size(11).color(theme::TEXT_FAINT), body].spacing(4).into()
                }
                None => body.into(),
            };
            list = list.push(row![face, container(body).width(Length::Fill)].spacing(12));
        }
        if let Some(who) = &t.running {
            list = list.push(
                row![claude_face(), text(format!("{who} is working…")).size(13).color(theme::TEXT_DIM)].spacing(12).align_y(Alignment::Center),
            );
        }
        let can_send = !t.busy() && !t.input.trim().is_empty();
        let composer = row![
            self.my_face(),
            text_input("Ask Claude to change something, or @opus, @fable…", &t.input)
                .on_input(|s| Message::Ai(AiMsg::Input(s)))
                .on_submit(Message::Ai(AiMsg::Send))
                .padding([4, 4])
                .size(14)
                .style(theme::bare),
            button(icon(Icon::Send, 13.0, if can_send { theme::ACCENT } else { theme::TEXT_FAINT }))
                .padding(3)
                .style(theme::flat(false))
                .on_press_maybe(can_send.then_some(Message::Ai(AiMsg::Send))),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        column![
            crate::view::vscroll(list).anchor_bottom().width(Length::Fill).height(Length::Fill),
            container(composer).padding(Padding { top: 10.0, right: 14.0, bottom: 10.0, left: 14.0 }),
        ]
        .into()
    }
}

/// Claude's picture in the gutter: the sparkle on a dark disc.
fn claude_face<'a>() -> Element<'a, Message> {
    container(icon(Icon::Sparkle, 13.0, Color::from_rgb8(0xd9, 0x77, 0x57)))
        .center(Length::Fixed(22.0))
        .style(|_: &Theme| container::Style {
            background: Some(theme::CARD_BG.into()),
            border: iced::Border { radius: 11.0.into(), color: theme::SELECTED, width: 1.0 },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_seen(msgs: &[hythe_git::threads::Event]) -> HashSet<String> {
        msgs.iter().map(|m| m.id.clone()).collect()
    }

    #[test]
    fn mentions_pick_the_agent() {
        let known = agents();
        assert_eq!(route("@opus can you review this?", &known), "opus");
        assert_eq!(route("hey @Fable, look", &known), "fable");
        assert_eq!(route("no mention", &known), "claude");
        assert_eq!(route("@nobody hi", &known), "claude");
    }

    #[test]
    fn preamble_tells_agents_how_to_reach_the_others() {
        let known = agents();
        let me = known.iter().find(|a| a.name == "fable").unwrap();
        let p = preamble(me, "t-1", &known);
        assert!(p.contains("You are `fable`"));
        assert!(p.contains("@opus") && !p.contains("@fable,") && p.contains("hythe-agent say"));
        assert!(p.contains("hythe-agent note") && p.contains("hythe-agent notes"));
    }

    /// A stand-in agent that echoes who it is, the thread, and what it was told.
    #[test]
    fn a_step_runs_the_agent_with_its_identity_and_records_both_sides() {
        let dir = std::env::temp_dir().join(format!("hythe-ai-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let o = Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(o.status.success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Tester"]);
        git(&["config", "user.email", "t@example.com"]);
        #[cfg(windows)]
        let (script, body) = (dir.join("fake.cmd"), "@echo off\r\nmore >nul\r\nif \"%HYTHE_AGENT_NAME%\"==\"fable\" hythe-agent say opus please review lib.rs >nul\r\necho {\"result\":\"I am %HYTHE_AGENT_NAME% in %HYTHE_THREAD%\",\"session_id\":\"abc-123\"}\r\n");
        #[cfg(not(windows))]
        let (script, body) = (dir.join("fake.sh"), "#!/bin/sh\ncat >/dev/null\n[ \"$HYTHE_AGENT_NAME\" = fable ] && hythe-agent say opus please review lib.rs >/dev/null\necho\"{\\\"result\\\":\\\"I am $HYTHE_AGENT_NAME in $HYTHE_THREAD\\\",\\\"session_id\\\":\\\"abc-123\\\"}\"\n");
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("HYTHE_AGENT_CMD", &script);
        // Where the real `hythe-agent` binary was built.
        let tools = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("debug");
        std::env::set_var("HYTHE_TOOLS_DIR", std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).map(|d| d.join("debug")).unwrap_or(tools));

        let def = agents().into_iter().find(|a| a.name == "opus").unwrap();
        let done = step(dir.clone(), "Review".into(), None, def, None, Some("@opus please review".into()), Vec::new(), HashSet::new());

        let thread = done.thread.clone().expect("thread was created");
        let (reply, session) = done.result.clone().unwrap();
        assert_eq!(reply, format!("I am opus in {thread}"));
        assert_eq!(session.as_deref(), Some("abc-123"));
        // The window is told about the agent's reply (not about the message it sent itself).
        assert_eq!(done.events.len(), 1);
        assert_eq!(done.events[0].agent.as_deref(), Some("opus"));
        // Both sides are in the store.
        let repo = hythe_git::Repo::open(&dir).unwrap();
        let msgs = repo.thread_messages(&thread).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].to, vec!["opus"]);
        assert!(done.next.is_none(), "opus did not message anyone");

        // Fable asks Opus for a review from inside its own run, using the real `hythe-agent`.
        let fable = agents().into_iter().find(|a| a.name == "fable").unwrap();
        let done = step(dir.clone(), "Review".into(), Some(thread.clone()), fable, None, Some("@fable ask opus to review".into()), Vec::new(), done_seen(&msgs));
        std::env::remove_var("HYTHE_AGENT_CMD");
        std::env::remove_var("HYTHE_TOOLS_DIR");
        let (to, mail) = done.next.clone().expect("fable's message to opus is waiting for opus");
        assert_eq!(to, "opus");
        assert_eq!(mail.len(), 1);
        assert!(mail[0].body.contains("please review lib.rs") && mail[0].agent.as_deref() == Some("fable"), "{:?}", mail[0]);
        // The window also learns that fable spoke to opus.
        assert!(done.events.iter().any(|e| e.to == vec!["opus".to_string()] && e.agent.as_deref() == Some("fable") && e.body.contains("please review")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
