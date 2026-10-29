//! End-to-end tests: real child processes on real pseudo-terminals.

use std::time::Duration;

use hythe_term::{Key, Mods, Palette, SpawnConfig, TerminalManager};

const WAIT: Duration = Duration::from_secs(20);

fn echo(text: &str) -> SpawnConfig {
    if cfg!(windows) {
        SpawnConfig::new("cmd.exe").args(["/C", &format!("echo {text}")])
    } else {
        SpawnConfig::new("sh").args(["-c", &format!("echo {text}")])
    }
}

fn exit_with(code: i32) -> SpawnConfig {
    if cfg!(windows) {
        SpawnConfig::new("cmd.exe").args(["/C", &format!("exit {code}")])
    } else {
        SpawnConfig::new("sh").args(["-c", &format!("exit {code}")])
    }
}

fn interactive_shell() -> SpawnConfig {
    if cfg!(windows) {
        SpawnConfig::new("cmd.exe")
    } else {
        SpawnConfig::new("sh")
    }
}

#[test]
fn runs_a_command_and_captures_output() {
    let mut mgr = TerminalManager::default();
    let id = mgr.spawn(&echo("hello-hythe")).unwrap();
    let s = mgr.get(id).unwrap();
    assert!(s.wait_for_text("hello-hythe", WAIT), "screen was: {:?}", s.snapshot().text());
    assert_eq!(s.wait_for_exit(WAIT), Some(Some(0)));
}

#[test]
fn reports_exit_codes() {
    let mut mgr = TerminalManager::default();
    let id = mgr.spawn(&exit_with(3)).unwrap();
    let s = mgr.get(id).unwrap();
    assert_eq!(s.wait_for_exit(WAIT), Some(Some(3)));
    assert!(!s.is_running());
    let info = &mgr.list()[0];
    assert!(!info.running);
    assert_eq!(info.exit_code, Some(3));
}

#[test]
fn interactive_input_and_resize() {
    let mut mgr = TerminalManager::default();
    mgr.set_default_size(80, 24);
    let id = mgr.spawn(&interactive_shell()).unwrap();
    let s = mgr.get(id).unwrap();

    let snap = s.snapshot();
    assert_eq!((snap.cols, snap.rows), (80, 24));

    // Typing goes through the key encoder, then the shell echoes and runs it.
    for ch in "echo typed-via-keys".chars() {
        assert!(s.send_key(&Key::Char(ch), Mods::default()));
    }
    s.send_key(&Key::Enter, Mods::default());
    assert!(s.wait_for_text("typed-via-keys", WAIT), "screen was: {:?}", s.snapshot().text());

    s.resize(120, 40, 8, 16);
    let snap = s.snapshot();
    assert_eq!((snap.cols, snap.rows), (120, 40));
    assert_eq!(snap.cells.len(), 120 * 40);

    s.write(b"echo after-resize\r".to_vec());
    assert!(s.wait_for_text("after-resize", WAIT));
    assert!(mgr.close(id));
    assert!(mgr.get(id).is_none());
}

#[test]
fn ansi_colours_resolve_through_the_palette() {
    let cfg = if cfg!(windows) {
        SpawnConfig::new("powershell").args([
            "-NoProfile",
            "-Command",
            "[Console]::Out.Write([char]27 + '[31mREDTEXT' + [char]27 + '[0m')",
        ])
    } else {
        SpawnConfig::new("sh").args(["-c", "printf '\\033[31mREDTEXT\\033[0m'"])
    };
    let mut mgr = TerminalManager::default();
    let id = mgr.spawn(&cfg).unwrap();
    let s = mgr.get(id).unwrap();
    assert!(s.wait_for_text("REDTEXT", WAIT), "screen was: {:?}", s.snapshot().text());

    let pal = Palette::default();
    let snap = s.snapshot();
    let cell = snap
        .cells
        .iter()
        .find(|c| c.ch == 'R')
        .expect("an R cell on screen");
    assert_eq!(cell.fg, pal.ansi[1], "red text should use ANSI red");
    assert_eq!(snap.cell(0, 0).bg, pal.bg);
}

#[test]
fn manager_runs_several_sessions_independently() {
    let mut mgr = TerminalManager::default();
    let a = mgr.spawn(&echo("session-a")).unwrap();
    let b = mgr.spawn(&echo("session-b")).unwrap();
    assert_ne!(a, b);
    let (sa, sb) = (mgr.get(a).unwrap(), mgr.get(b).unwrap());
    assert!(sa.wait_for_text("session-a", WAIT));
    assert!(sb.wait_for_text("session-b", WAIT));
    assert!(!sa.snapshot().text().contains("session-b"));
    assert_eq!(mgr.list().len(), 2);
    assert!(!mgr.take_dirty().is_empty());
}

#[test]
fn missing_program_is_an_error_or_exits_nonzero() {
    let mut mgr = TerminalManager::default();
    match mgr.spawn(&SpawnConfig::new("definitely-not-a-real-program-xyz")) {
        Err(_) => {}
        Ok(id) => {
            let s = mgr.get(id).unwrap();
            let code = s.wait_for_exit(WAIT).expect("process should exit");
            assert_ne!(code, Some(0));
        }
    }
}
