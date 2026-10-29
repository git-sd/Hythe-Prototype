# hythe-relay

A self-hostable relay for real-time collaborative editing. No third-party
service: you run the server, and clients connect over WebSocket.

```
cargo run -p hythe-relay                        # listens on 127.0.0.1:7878
cargo run -p hythe-relay -- --listen 0.0.0.0:9000
```

In the editor, open **Collaborate** (the people icon in the bottom bar, or
**Share** in the title bar), enter the server address, a room name and your
name, and press **Start sharing**. Everyone who joins the same room and opens
the same file (matched by its path inside the repository) edits it together
and sees each other's cursors.

## How it works

- Documents are CRDTs (`yrs`), so concurrent edits merge instead of conflicting.
- The first person to open a file seeds it with their text; everyone after
  adopts the shared text. Two people opening at the same moment never both seed.
- The server keeps each document's state while anyone is in the room, so a
  late joiner gets the current text. Nothing is written to disk on the server.
- Cursors are relayed but not stored.

## Library pieces

- `SharedDoc`: a text CRDT with UTF-16 offsets and a list of changes to mirror
  into a plain-text editor.
- `Client`: connects on its own thread; poll it from a UI loop.
- `server::serve`: embed the server in your own program.

Try it with a scripted collaborator:

```
cargo run -p hythe-relay --example peer -- ws://127.0.0.1:7878 myroom src/main.rs Robin
```

## Limits

- No authentication or encryption. Use it on a trusted network or behind a
  TLS-terminating proxy.
- A remote edit clears the receiving editor's undo history for that file.
- Only text files under the repository are shared.
