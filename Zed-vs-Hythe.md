# Zed vs. Hythe (as of 2026-11-03)

Zed is a mature, shipping code editor. Hythe is an early prototype that borrows
Zed's look (dark theme sampled from Zed, IBM Plex Sans/Mono, Zed-style diff and
edit cards) but is built around a different idea: a **team-and-AI conversation
thread next to your code**.

## At a glance

| | Zed | Hythe (this repo) |
|---|---|---|
| Status | Production editor, large team, public releases | Prototype: ~9.5k lines in `src/` plus 4 small crates, 50 commits |
| Language / GUI | Rust, its own GPU UI framework (GPUI) | Rust, `iced` 0.14 |
| Centre of gravity | The editor and language tooling | The thread column: team chat, Claude threads, commits and comments |
| Platforms | macOS, Linux, Windows | Desktop only (nothing packaged or tested beyond local builds) |

## Where they are alike

- Native Rust app, fast start-up, dark theme and Plex typefaces.
- Tree-sitter syntax highlighting (Hythe bundles ~17 grammars: Rust, Python, JS/TS, Go, C/C++, Java, C#, JSON, TOML, HTML, CSS, Bash, Ruby, PHP).
- Multi-pane editing, draggable tabs, per-tab zoom, built-in terminal, command palette, file explorer.
- Diff cards for changes and edit cards for code blocks in messages.

## Where Hythe differs

**Conversation first.** The right-hand column (a third of the window, dockable
either side or poppable into its own window) shows the team conversation. You
can leave comment threads on messages and quote a specific phrase, which is
highlighted in the message.

**Claude threads as conversations.** A thread talks to the Claude Code CLI; the
edits it makes land in the *Changes* view as diff cards you can review.

**Source control in the same feed.** Commits appear as cards in the feed, can be
expanded in place, and you can commit from the message box. Git goes through the
real `git` command line (crate `hythe-git`), so hooks and credentials behave as
in your terminal.

**Built-in collaboration without a third-party service.** `hythe-relay` is a
self-hostable WebSocket relay that forwards CRDT updates between editors in a
room. Zed's collaboration uses Zed's hosted service.

**Offline completion, no language server.** `hythe-complete` suggests keywords,
standard-library names, snippets, members after `.` / `::`, and symbols from open
documents, for 8 languages. Zed uses language servers (LSP) for completion,
go-to-definition, diagnostics and more.

**Terminals and agents.** `hythe-term` runs shells and CLI agents (such as
Claude Code) on a pseudo-terminal with profiles.

## What Zed has that Hythe does not (yet)

- Language servers: diagnostics, go-to-definition, rename, hover, inlay hints.
- Mature editing: multi-cursor, vim mode, rich search/replace, code folding, snippets, formatters.
- Extension ecosystem and themes; a debugger; a remote development story.
- Polished, hosted collaboration (voice, shared projects) and built-in AI features with model choice.
- Settings, keybinding customisation, accessibility and a stable release process.
- Years of performance and reliability work on large files and projects.

## Summary

Zed is the better editor today by a wide margin. Hythe is an experiment in what
an editor looks like when the team/AI conversation, not the file tree, is the
main surface, wearing Zed's visual style.

*Note: the Zed column is from general knowledge of Zed and may be out of date;
the Hythe column is from this repository's source and commit history.*
