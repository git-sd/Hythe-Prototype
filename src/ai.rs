//! Threads with Claude, drawn as a conversation like Zed's agent panel.
//!
//! Each message runs the Claude Code CLI in the project folder (`claude -p`, the
//! prompt on stdin, JSON out) and resumes the same session for follow-ups. File
//! edits are accepted, so they land in the working tree and show up in the
//! Changes tab. `HYTHE_AGENT_CMD` names another program to run instead, for tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding, Task, Theme};

use crate::icons::{icon, Icon};
use crate::{theme, App, Message};

pub struct AiThread {
    /// The project the thread belongs to.
    pub root: PathBuf,
    pub title: String,
    /// Claude's session id, once the first reply arrived.
    pub session: Option<String>,
    pub turns: Vec<Turn>,
    pub busy: bool,
    pub input: String,
}

pub struct Turn {
    pub from_you: bool,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum AiMsg {
    New,
    Select(usize),
    Input(String),
    Send,
    /// Claude answered thread `n`: (reply, session id), or why it could not.
    Reply(usize, Result<(String, Option<String>), String>),
}

/// Validate whether a command name/path from `HYTHE_AGENT_CMD` is safe to spawn.
pub fn is_safe_command_name(name: &str) -> bool {
    let s = name.trim();
    if s.is_empty() {
        return false;
    }
    // Reject shell control characters and metacharacters
    const FORBIDDEN: &[char] = &[';', '&', '|', '>', '<', '$', '`', '\n', '\r', '\0', '(', ')', '{', '}', '*', '?'];
    if s.contains(FORBIDDEN) {
        return false;
    }
    true
}

/// Sanitize error text from external AI processes to avoid leaking private paths or tokens.
pub fn sanitize_error(raw: &str) -> String {
    let line = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("no reply").trim();
    let mut s = line.to_string();

    // Redact home directories
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home_str = home.to_string_lossy();
        if !home_str.is_empty() {
            s = s.replace(&*home_str, "~");
        }
    }

    // Redact common secret / token patterns
    if let Some(pos) = s.find("sk-") {
        let end = s[pos..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'').map(|e| pos + e).unwrap_or(s.len());
        s.replace_range(pos..end, "[REDACTED_KEY]");
    }

    // Cap length to prevent unbounded string allocations
    if s.len() > 200 {
        s.truncate(197);
        s.push_str("...");
    }

    if s.is_empty() {
        "no reply".into()
    } else {
        s
    }
}

/// Safely parse Claude JSON output or fallback to sanitized error message.
pub fn parse_ai_response(stdout: &str, stderr: &str) -> Result<(String, Option<String>), String> {
    let val = serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .or_else(|| {
            if let (Some(start), Some(end)) = (stdout.find('{'), stdout.rfind('}')) {
                if start < end {
                    serde_json::from_str::<serde_json::Value>(&stdout[start..=end]).ok()
                } else {
                    None
                }
            } else {
                None
            }
        });

    match val {
        Some(v) => {
            let result = v.get("result")
                .and_then(|r| r.as_str())
                .or_else(|| v.get("error").and_then(|e| e.as_str()))
                .or_else(|| v.get("message").and_then(|m| m.as_str()))
                .unwrap_or("")
                .to_string();
            let session = v.get("session_id").and_then(|s| s.as_str()).map(str::to_string);
            if v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false) {
                let msg = if result.is_empty() { "Claude reported an error".into() } else { sanitize_error(&result) };
                Err(msg)
            } else {
                Ok((result, session))
            }
        }
        None => {
            let err = stderr.trim();
            let raw = if !err.is_empty() { err } else { stdout.trim() };
            Err(sanitize_error(raw))
        }
    }
}

/// Ask Claude in `root`, continuing `session` if there is one.
pub fn run(root: &Path, prompt: &str, session: Option<&str>) -> Result<(String, Option<String>), String> {
    let raw_program = std::env::var("HYTHE_AGENT_CMD").unwrap_or_else(|_| "claude".into());
    let program = if is_safe_command_name(&raw_program) {
        raw_program
    } else {
        "claude".into()
    };
    let mut args = vec!["-p", "--output-format", "json", "--permission-mode", "acceptEdits"];
    // Only ever pass back an id Claude gave us (hex and dashes).
    if let Some(s) = session.filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')) {
        args.extend(["--resume", s]);
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
    let mut cmd = Command::new(&program);
    cmd.args(&args).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not start {program}: {e}"))?;
    // The prompt goes on stdin, never on the command line.
    if let Some(mut pipe) = child.stdin.take() {
        let prompt = prompt.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(prompt.as_bytes());
        });
    }
    let out = child.wait_with_output().map_err(|e| sanitize_error(&e.to_string()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    parse_ai_response(&stdout, &stderr)
}

impl App {
    /// The Claude thread showing in the thread column, if any.
    pub(crate) fn ai_thread(&self) -> Option<&AiThread> {
        let root = self.root.as_ref()?;
        self.ai_active.and_then(|i| self.ai.get(i)).filter(|t| t.root == root.path)
    }

    pub(crate) fn ai_update(&mut self, msg: AiMsg) -> Task<Message> {
        match msg {
            AiMsg::New => {
                let Some(root) = self.root.as_ref().map(|r| r.path.clone()) else { return Task::none() };
                self.ai.push(AiThread { root, title: "New thread".into(), session: None, turns: Vec::new(), busy: false, input: String::new() });
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
                if prompt.is_empty() || t.busy {
                    return Task::none();
                }
                if t.turns.is_empty() {
                    t.title = crate::chat::excerpt(&prompt);
                }
                t.turns.push(Turn { from_you: true, text: prompt.clone() });
                t.input.clear();
                t.busy = true;
                let (root, session) = (t.root.clone(), t.session.clone());
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || run(&root, &prompt, session.as_deref()))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |r| Message::Ai(AiMsg::Reply(i, r)),
                );
            }
            AiMsg::Reply(i, result) => {
                if let Some(t) = self.ai.get_mut(i) {
                    t.busy = false;
                    match result {
                        Ok((reply, session)) => {
                            t.turns.push(Turn { from_you: false, text: reply });
                            if session.is_some() {
                                t.session = session;
                            }
                        }
                        Err(e) => t.turns.push(Turn { from_you: false, text: format!("Couldn't reach Claude: {e}") }),
                    }
                }
                // Claude may have edited files: bring the Changes tab and open files up to date.
                self.refresh_git();
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
            let face = if turn.from_you { self.my_face() } else { claude_face() };
            let mut body = column![].spacing(8);
            for part in crate::chat::split_code(&turn.text) {
                body = body.push(match part {
                    crate::chat::Part::Text(s) => text(s).size(14).line_height(1.45).into(),
                    crate::chat::Part::Code { info, lines } => crate::chat::code_card(info, lines),
                });
            }
            list = list.push(row![face, body.width(Length::Fill)].spacing(12));
        }
        if t.busy {
            list = list.push(
                row![claude_face(), text("Working…").size(13).color(theme::TEXT_DIM)].spacing(12).align_y(Alignment::Center),
            );
        }
        let can_send = !t.busy && !t.input.trim().is_empty();
        let composer = row![
            self.my_face(),
            text_input("Ask Claude to change something", &t.input)
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

    #[test]
    fn command_injection_safety_validation() {
        assert!(is_safe_command_name("claude"));
        assert!(is_safe_command_name("/usr/local/bin/claude"));
        assert!(is_safe_command_name("C:\\tools\\claude.exe"));

        // Reject dangerous characters
        assert!(!is_safe_command_name("claude; rm -rf /"));
        assert!(!is_safe_command_name("claude && evil"));
        assert!(!is_safe_command_name("claude | curl evil.com"));
        assert!(!is_safe_command_name("claude > out.txt"));
        assert!(!is_safe_command_name("claude $(whoami)"));
        assert!(!is_safe_command_name("claude `whoami`"));
        assert!(!is_safe_command_name(""));
        assert!(!is_safe_command_name("   "));
    }

    #[test]
    fn error_sanitization_removes_secrets_and_home_paths() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
        let raw = format!("Error reading {}/private/config: invalid token sk-1234567890abcdef12345678", home);
        let sanitized = sanitize_error(&raw);
        assert!(!sanitized.contains(&home), "Home directory must be redacted");
        assert!(sanitized.contains("~/private/config"));
        assert!(!sanitized.contains("sk-1234567890abcdef12345678"), "API key must be redacted");
        assert!(sanitized.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn parse_ai_response_handles_clean_and_embedded_json() {
        // Clean JSON
        let clean = r#"{"result": "Hello world", "session_id": "sess-123", "is_error": false}"#;
        let res = parse_ai_response(clean, "").unwrap();
        assert_eq!(res.0, "Hello world");
        assert_eq!(res.1, Some("sess-123".into()));

        // Embedded JSON with CLI preamble warnings
        let embedded = "Warning: node experimental feature enabled\n{\"result\": \"reply with warning\", \"is_error\": false}";
        let res2 = parse_ai_response(embedded, "").unwrap();
        assert_eq!(res2.0, "reply with warning");

        // Error JSON with error field
        let err_json = r#"{"error": "Rate limit exceeded", "is_error": true}"#;
        let err_res = parse_ai_response(err_json, "").unwrap_err();
        assert_eq!(err_res, "Rate limit exceeded");

        // Malformed non-JSON output falls back to sanitized stderr/stdout
        let bad_out = "Uncaught exception in /tmp/test.js";
        let err_res2 = parse_ai_response(bad_out, "").unwrap_err();
        assert_eq!(err_res2, "Uncaught exception in /tmp/test.js");
    }
}

