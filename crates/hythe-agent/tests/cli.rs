//! Two agents talk through the real `hythe-agent` binary in a throwaway repo.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn agent(dir: &Path, name: &str, thread: &str, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hythe-agent"))
        .current_dir(dir)
        .env("HYTHE_AGENT_NAME", name)
        .env("HYTHE_THREAD", thread)
        .args(args)
        .output()
        .unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn agents_exchange_messages_and_notes() {
    let dir = std::env::temp_dir().join(format!("hythe-agent-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "t@example.com"]);

    let thread = "t-0000000000001-abc";
    let (ok, out) = agent(&dir, "fable", thread, &["say", "opus", "please", "review", "lib.rs"]);
    assert!(ok, "{out}");

    // Opus sees it once, then the inbox is empty.
    let (ok, out) = agent(&dir, "opus", thread, &["inbox"]);
    assert!(ok && out.contains("please review lib.rs") && out.contains("fable"), "{out}");
    let (_, out) = agent(&dir, "opus", thread, &["inbox"]);
    assert!(out.contains("no new messages"), "{out}");
    // Someone else never saw it.
    let (_, out) = agent(&dir, "sonnet", thread, &["inbox"]);
    assert!(out.contains("no new messages"), "{out}");

    // Opus leaves a note; Fable reads it.
    let (ok, _) = agent(&dir, "opus", thread, &["note", "Review Findings", "Looks", "good,", "one", "nit"]);
    assert!(ok);
    let (_, out) = agent(&dir, "fable", thread, &["notes"]);
    assert!(out.contains("review-findings") && out.contains("opus"), "{out}");
    let (_, out) = agent(&dir, "fable", thread, &["notes", "review-findings"]);
    assert!(out.contains("one nit"), "{out}");

    let (_, out) = agent(&dir, "fable", thread, &["read"]);
    assert!(out.contains("fable -> opus"), "{out}");

    // Outside Hythe there is no identity, and the command says so.
    let out = Command::new(env!("CARGO_BIN_EXE_hythe-agent")).current_dir(&dir).env_remove("HYTHE_AGENT_NAME").arg("inbox").output().unwrap();
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}
