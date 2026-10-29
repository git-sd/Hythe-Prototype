# hythe-term

A fast terminal backend for running shells and CLI agents (Claude Code, Codex,
Gemini CLI, Aider, …) from a Rust program. It has no UI dependency: you get
a screen snapshot and render it however you like.

Built on [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal)
for the emulator and PTY layer (ConPTY on Windows, `openpty` on Unix).

```rust
use hythe_term::{available_agents, Key, Mods, Profile, TerminalManager};

let mut mgr = TerminalManager::default();
let shell = mgr.spawn_profile(&Profile::shell(), None)?;

let session = mgr.get(shell).unwrap();
session.write(b"echo hello\r".to_vec());
session.wait_for_text("hello", std::time::Duration::from_secs(5));

// Render this however you like: rows x cols of resolved cells.
let screen = session.snapshot();
println!("{}", screen.text());

// Launch whichever agents are installed.
for agent in available_agents() {
    mgr.spawn_profile(&agent, Some("path/to/project".into()))?;
}
```

## What's in it

- **`Session`**: one child process on a PTY plus its emulated screen. A
  dedicated I/O thread feeds output into the emulator; `take_dirty()` is a
  cheap atomic check and `snapshot()` copies the visible grid only when you ask.
- **`TerminalManager`**: owns many sessions, lists them, closes them.
- **`Profile` / `SpawnConfig`**: how to launch a shell or an agent. Programs
  are resolved on `PATH`; on Windows npm-installed `.cmd` shims are run
  through `cmd.exe` automatically.
- **Input**: `encode_key` / `encode_paste` produce the right escape sequences,
  including application-cursor mode, modifiers, and bracketed paste.
- **Selection and scrollback** (10,000 lines), window-title tracking, exit codes.

## Performance

Measured on one Windows 11 machine (release build, 200x50 grid, 100 MB of
coloured text):

| What | Result |
| --- | --- |
| Emulator alone (`examples/parse_only`) | ~106 MB/s |
| Through a PTY, child writing in large chunks | ~9 MB/s |
| Through a PTY, `cmd /C type file` | ~2 MB/s |
| Snapshot of a 200x50 grid | ~150 µs |

The emulator is not the limit. End to end, throughput is bound by Windows'
ConPTY layer and by how the child process writes. Unix numbers are not measured
yet. Reproduce with `cargo run --release --example throughput -- 100`.

## Status

Early. Tested on Windows; the Unix code path compiles against the same
`alacritty_terminal` API but has not been run. Mouse reporting to applications
and kitty keyboard protocol are not implemented.
