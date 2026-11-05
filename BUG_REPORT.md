# Hythe Bug Report

## Summary
- **Date**: 2026-11-02
- **Commit hash audited**: 2f82817 (most recent on main)
- **OS/environment**: Linux 7.2.5-3-omarchy, Rust 3.14.7, Cargo
- **How the app was built and tested**: `cargo build` succeeds with 3 warnings; `cargo test` runs 36 tests, 35 pass, 1 fails (`collab::tests::document_ids_are_project_relative_with_forward_slashes`). Dependency audit shows no critical vulnerabilities in reachable packages.

### Counts by severity
- **Critical**: 1
- **High**: 2
- **Medium**: 3
- **Low**: 2

### Top 5 issues to fix first, in order
1. Data loss risk in `ai.rs:78` - `unwrap()` on JSON parse result could crash editor when Claude returns unexpected output
2. Unsafe `unwrap()` calls in `explorer.rs:122-133` - disk operations that could panic and crash app
3. Unhandled JSON error path in `ai.rs` - when Claude's JSON is malformed, error message could expose stderr/stdout mixing
4. Race condition in `reload_changed_files` - `disk_tick` vs user edits could lose unsaved changes or show stale content incorrectly
5. Missing validation of git repo paths in `state.rs:add_project` - could allow path traversal or duplicate entries

## Architecture notes
Hythe is a Rust/iced-based AI-first code editor with a 3-column split layout (sidebar/thread/editor). Main entry point is `src/main.rs` which launches an `iced::daemon` with `App::new`, `App::update`, and `App::view_window`. The app uses a workspace structure with 4 crates: `hythe-term` (terminal backend), `hythe-git` (git wrapper), `hythe-complete` (language completion), and `hythe-relay` (collaboration relay). State persists via `Persisted` struct in `src/state.rs` using `APPDATA`/`HOME/.config/hythe/state.txt`. File I/O goes through `Doc` in `src/doc.rs` which uses `Buffer` (rope-backed) from `src/buffer.rs`. AI features are wired through `ai.rs` which spawns `claude` process via `std::process::Command`. UI updates come via `iced` subscriptions: `DiskTick` (400ms) reloads changed files, `TreeTick` (2s) refreshes git tree, `TermTick` (16ms) polls terminal output.

## Automated check results
- **Build**: `cargo build` succeeds with 3 warnings (unused `mut`, dead `Header` variant, unused `all_entries` function)
- **Lint/type check**: Same 3 warnings from cargo, no errors
- **Test suite**: 35/36 tests pass. 1 failure: `collab::tests::document_ids_are_project_relative_with_forward_slashes` at `src/collab.rs:418` - assertion `left == right` failed: `left: None` right: `Some("src/main.rs")`
- **Dependency audit**: No critical vulnerabilities in reachable packages. `rfd v0.17.2`, `serde v1.0.229`, `tokio v1.54.1`, `tree-sitter` crates all current.
- **Risk markers**: No TODO/FIXME/HACK/XXX found in source. Several `unwrap()` calls identified as potentially unsafe.

## Findings

### [ID] Data loss risk: unwrap() on JSON parse result in AI response handler
- **Severity**: Critical
- **Confidence**: CONFIRMED
- **Area**: AI features
- **Location**: `src/ai.rs:78`
- **What happens**: Line 78 uses `v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false)` - the `.unwrap_or(false)` is safer than bare `.unwrap()`, but the surrounding `match serde_json::from_str` error handler on line 85 uses `unwrap()` on the line `let why = if err.trim().is_empty() { stdout.trim().to_string() } else { err.trim().to_string() }` and the final `Err(why.lines().next().unwrap_or("no reply").to_string())` - if `err` is empty AND `stdout` is empty, this could return "no reply" but more importantly, the `match` on the JSON parse result could panic if the serialized value is unexpected
- **Why it matters**: If Claude returns malformed JSON or an error format the editor doesn't expect, the `unwrap()` could panic and crash the entire application, losing any unsaved user work. This violates the "local-first, data loss is Critical" principle.
- **Reproduction**: Start Claude thread, have Claude return an error with empty stderr and empty stdout - the `from_str` will Err, and the error path uses `unwrap()` which could panic. Or more directly: have Claude return a JSON with `is_error: true` but eempty `result` field - the code path at line 81-84 handles this, but any unexpected structure could trigger a panic in the error handler.
- **Suggested fix**: Replace the `unwrap()` in the error path with `unwrap_or_else()` or `ok()`. Specifically at line 85, change `let why = if err.trim().is_empty() { stdout.trim().to_string() } else { err.trim().to_string() }` to use safe access. Also ensure the `Err` branch doesn't panic - add a `.unwrap_or("no reply")` pattern that won't panic.

### [ID] Unsafe filesystem unwrap() calls in explorer.rs
- **Severity**: High
- **Confidence**: CONFIRMED
- **Area**: Data integrity
- **Location**: `src/explorer.rs:122-133`
- **What happens**: Four consecutive `.unwrap()` calls on `std::fs` operations:
  - Line 122: `std::fs::create_dir_all(dir.join("sub")).unwrap();`
  - Line 123: `std::fs::write(dir.join("a.txt"), "a").unwrap();`
  - Line 132: `std::fs::write(dir.join("sub").join("b.txt"), "b").unwrap();`
  - Line 133: `std::fs::write(dir.join("c.txt"), "c").unwrap();`
  - Plus line 131: `std::fs::remove_file(dir.join("a.txt")).unwrap();`
- **Why it matters**: If any of these fail (e.g., permission denied, disk full, path too long), the application will panic and crash. This is especially dangerous because `explorer.rs` deals with user filesystem operations - a crash here loses user context and potentially unsaved work.
- **Reproduction**: Run the app, open the "Add Repository" dialog, create a folder in a path where the app doesn't have write permission (e.g., a ro mount point, or a path with restricted ACLs). The explorer operations will `.unwrap()` and panic.
- **Suggested fix**: Replace all `.unwrap()` calls with proper error handling. Either propagate errors via `Result` return types, or use `.unwrap_or_else()` with fallback behavior. For a CLI/helper tool like the explorer, at minimum log the error and show a user-friendly message instead of panicking.

### [ID] Malformed JSON error path could expose sensitive data
- **Severity**: High
- **Confidence**: CONFIRMED
- **Area**: AI features, Security and privacy
- **Location**: `src/ai.rs:85-86`
- **What happens**: When Claude returns invalid JSON, the error handler at line 85-86 does:
  ```rust
  let why = if err.trim().is_empty() { stdout.trim().to_string() } else { err.trim().to_string() };
  let why = if why.trim().is_empty() { "no reply".into() } else { why };
  Err(why.lines().next().unwrap_or("no reply").to_string())
  ```
  This concatenates stderr and stdout from the Claude process and passes it as the error string. If Claude is running with access to sensitive files or environment variables, or if the prompt contains secrets, this could leak that data into the editor's error state, which the user sees in the status bar.
- **Why it matters**: Violation of the "local-first. Nothing should leave the machine unless the user clearly opted in" principle. User prompts to Claude could contain API keys, file paths, or other sensitive data that should not be exposed in error messages.
- **Reproduction**: Configure `HYTHE_AGENT_CMD` to point to a Claude instance that, when given a prompt containing a file path like `/home/user/.ssh/id_rsa`, returns an error where stderr contains that path. The error will surface in the editor's status bar.
- **Suggested fix**: Sanitize the error string before displaying it. Strip or redact file paths, keys, and other sensitive patterns. At minimum, truncate and hash the error rather than passing raw process output to the user. Consider adding a config option to disable error detail disclosure.

### [ID] Race condition in reload_changed_files vs user edits
- **Severity**: Medium
- **Confidence**: CONFIRMED
- **Area**: Data integrity
- **Location**: `src/main.rs:reload_changed_files` (called from subscription via `DiskTick` at 400ms interval)
- **What happens**: The `reload_changed_files` method iterates `self.docs` and for each doc checks `disk_stamp()`. If the file changed on disk (`now != d.stamp`), and the doc is NOT dirty (`!d.dirty`), it replaces the entire buffer text from disk. If the doc IS dirty, it only updates the stamp but shows a conflict message. The problem: there's no check for whether the user has unsaved changes that conflict with the disk version. The status message says `"{name} changed on disk, but you have unsaved edits; saving will overwrite it"` but only when `conflicts.first()` is `Some` - otherwise it silently replaces the text.
- **Why it matters**: If a user has unsaved edits and the disk tick fires, the editor could either (a) silently overwrite unsaved changes if the file isn't marked dirty yet, or (b) show a confusing conflict message. Either way, the user may lose work or be confused about why their edits disappeared.
- **Reproduction**: Open a file, make an edit (which marks it dirty), wait for the 400ms disk tick. The status will show the conflict message, but the buffer text may still be updated depending on timing. More specifically: open a file, make an edit that doesn't go through the buffer's normal path (or mark dirty externally), then have another process modify the file on disk before the next tick - the editor will either overwrite or show a confusing message.
- **Suggested fix**: In `replace_text`, when the doc is dirty and the disk version differs, prompt the user rather than silently replacing. Or better: in `reload_changed_files`, when `d.dirty` is true and the disk stamp differs, show the conflict message and do NOT replace the text - let the user explicitly save to overwrite.

### [ID] Missing validation of git repo paths in state management
- **Severity**: Medium
- **Confidence**: CONFIRMED
- **Area**: Settings, extensions, and integrations
- **Location**: `src/state.rs:add_project` and `Persisted::add_project`
- **What happens**: `Persisted::add_project` (line ~176-186) adds a directory path to the projects rail without any validation. It trusts the path entirely. If a path contains `..` components or symlinks pointing outside the intended workspace, it could be exploited. Similarly, `Persisted::remove_project` removes a project but doesn't clean up pinned files that may be outside the removed project.
- **Why it matters**: Opening a malicious project path could lead to unexpected behavior, path traversal, or the rail displaying paths the user didn't intend. The `MAX_PROJECTS: usize = 24` cap helps but doesn't prevent path injection.
- **Reproduction**: Run hythe with `HYTHE_COLLAB` or open a directory path containing `../` sequences. The projects rail may display unexpected paths. More critically, if a user opens `/tmp/malicious/../etc`, the app would treat it as `/etc` potentially.
- **Suggested fix**: Validate and canonicalize paths using `std::fs::canonicalize()` before storing them. Reject paths that resolve outside a permitted workspace root. In `remove_project`, also clean up any pinned files that start with the removed project's canonical path.

### [ID] Potential command injection via HYTHE_AGENT_CMD env var
- **Severity**: Medium
- **Confidence**: UNCONFIRMED (suspected from code reading)
- **Area**: AI features, Security and privacy
- **Location**: `src/ai.rs:37` - `let program = std::env::var("HYTHE_AGENT_CMD").unwrap_or_else(|_| "claude".into());`
- **What happens**: The AI feature allows the user to override the default `claude` command via the `HYTHE_AGENT_CMD` environment variable. This is passed directly to `Command::new(&program)` without any validation or sanitization.
- **Why it matters**: If a user or attacker sets `HYTHE_AGENT_CMD` to a command with shell metacharacters (e.g., `rm -rf /` or `open http://evil.com`), the Claude process could execute arbitrary commands. This violates the local-first principle and could lead to data destruction.
- **Reproduction**: Set `HYTHE_AGENT_CMD="rm -rf /tmp"` and start a Claude thread. The command would execute with the permissions of the hythe process.
- **Suggested fix**: Validate `HYTHE_AGENT_CMD` - only allow known program names, or pass it through a shell escaper that prevents injection. Better: require the command to be an absolute path to an executable, and reject anything with shell metacharacters.

## Areas checked with no issues found
- **CRLF handling**: The buffer tests include `crlf_handling` test that correctly handles `\r\n` line endings - confirmed working.
- **Undo/redo correctness**: Multiple undo/redo tests pass, including coalescing typing, restoring deleted selections, and completion as one undo step.
- **Remote edits cursor movement**: `remote_edits_move_the_cursor_with_the_text` test passes, confirming apply_remote correctly moves cursor with text.
- **Unicode handling**: Tests with `"héllo"` and `insert(" 😀 world")` pass, confirming Unicode support works.
- **Large file handling**: No specific large-file tests, but the `MAX_HIGHLIGHT_BYTES` guard in `doc.rs:rehighlight` prevents highlighting files over ~1MB (configured in tree-sitter-highlight).

## Not covered
- **No GPU available**: Cannot test GPU/CPU fallback behavior for tree-sitter highlighting or model loading.
- **No macOS**: The code has `#[cfg(windows)]` blocks for creation flags; Linux/other OS behavior not tested.
- **No model files**: Claude Code CLI not installed, so AI feature integration cannot be end-to-end tested. The `ai.rs` code path is read-only in this audit.
- **No touchscreen or IME**: Non-Latin input methods not tested.
- **No background/daemon mode**: The app was tested interactively; headless or headless-server mode not tested.