# hythe-relay

A self-hostable relay for real-time collaborative editing. No third-party
service: you run the server, and clients connect over WebSocket.

```
cargo run -p hythe-relay                        # listens on 127.0.0.1:7878, in memory
cargo run -p hythe-relay -- --listen 0.0.0.0:9000 --token SECRET --data-dir ./relay-data
```

In the editor, open **Collaborate** (**Share** in the title bar), enter the
server address, a room name, your name (and the password, if the server has
one), and press **Start sharing**. Everyone who joins the same room and opens
the same file (matched by its path inside the repository) edits it together
and sees each other's cursors.

## Options

| Flag | Meaning |
| --- | --- |
| `-l, --listen ADDR:PORT` | Address to listen on (default `127.0.0.1:7878`). |
| `--data-dir DIR` | Save every document under `DIR` and reload it on start. Without it, documents live in memory only. |
| `--token SECRET` | Every client must send this password. Also read from `HYTHE_RELAY_TOKEN`, which keeps it out of `ps`. |
| `--room-token ROOM=SECRET` | Password for one room only (repeatable). Overrides `--token` for that room. |
| `--room-ttl SECONDS` | How long an empty room stays in memory after its last peer leaves (default 3600). With `--data-dir` it is reloaded from disk on the next join. |
| `--max-message-mb N` | Largest message accepted (default 16). |
| `--max-peers N` | Peers per room (default 64). |

With no token configured the relay is open, as before; clients that send a
password to an open relay are accepted. A wrong or missing password gets a
clear message in the editor and is not retried.

## How it works

- Documents are CRDTs (`yrs`), so concurrent edits merge instead of conflicting.
- The first person to open a file seeds it with their text; everyone after
  adopts the shared text. The claim is taken atomically on the server, so two
  people opening at the same moment never both seed; if the seeder leaves
  before sending its text, the claim passes to the next person waiting. An
  empty file counts as seeded too.
- Rooms are kept after the last peer leaves (see `--room-ttl`). With
  `--data-dir`, each changed document is snapshotted about once a second
  (write to a temp file, fsync, rename, so a crash never leaves a torn file)
  and again on Ctrl-C.
- Cursors are relayed but not stored.

### Reconnecting

The client reconnects by itself with exponential backoff (0.5 s up to 15 s).
After each reconnect it says hello again and re-opens its documents. Edits
made while offline are kept in the local CRDT; once the server answers, the
client merges the server's state and sends its own whole state back. Applying
the same CRDT update twice does nothing, so offline edits merge without
duplicating text, whether the relay kept its documents (`--data-dir`) or was
restarted empty. The editor shows "Connection lost. Reconnecting..." and then
"Reconnected".

### Liveness and limits

- The server pings every 15 s and drops a connection that has been silent for
  45 s; the client does the same in the other direction (10 s / 35 s). Peers
  are told promptly when someone's connection dies.
- Messages larger than `--max-message-mb`, more than ~200 messages per second
  from one client, bad names, and full rooms are refused.
- Each client has a bounded outgoing queue. A client that stops reading is
  disconnected (it will reconnect and resync) instead of slowing everyone down.

## Running it on the internet (TLS)

The relay does not speak TLS itself. Put a reverse proxy in front that
terminates TLS and forwards WebSocket traffic, then use a `wss://` address in
the editor (the client accepts `ws://`, `wss://`, `http(s)://`, or a bare
`host[:port]`; trusted roots are bundled, no system certificates needed).

Keep the relay bound to `127.0.0.1` and set a token:

```
HYTHE_RELAY_TOKEN=choose-a-long-secret hythe-relay --listen 127.0.0.1:7878 --data-dir /var/lib/hythe-relay
```

### Caddy (automatic free HTTPS)

Point a DNS name (`relay.example.com`) at your server, open ports 80 and 443,
and use this `Caddyfile`:

```
relay.example.com {
    reverse_proxy 127.0.0.1:7878
}
```

Caddy obtains and renews a Let's Encrypt certificate on its own and proxies
WebSocket upgrades with no extra configuration. In the editor, set the relay
server to `wss://relay.example.com`.

### Cloudflare Tunnel (no open ports, no domain setup on your server)

```
cloudflared tunnel --url http://127.0.0.1:7878          # quick, random *.trycloudflare.com address
```

or a named tunnel on your own domain (`cloudflared tunnel create hythe`, then
route `relay.example.com` to `http://127.0.0.1:7878` in the tunnel config).
Use the resulting `wss://...` address in the editor. WebSockets work through
tunnels by default.

### nginx

```
location / {
    proxy_pass http://127.0.0.1:7878;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 3600s;
}
```

## Library pieces

- `SharedDoc`: a text CRDT with UTF-16 offsets and a list of changes to mirror
  into a plain-text editor.
- `DocSession`: the open/seed/adopt/resync state machine for one document
  (no I/O), used by the editor and the tests.
- `Client`: connects on its own thread and reconnects (`Client::connect_with`
  and `Options`); poll it from a UI loop. `Client::connect` is the one-shot
  variant that never retries.
- `server::{Relay, ServerConfig, serve_with}`: embed the server in your own program.

Try it with a scripted collaborator:

```
cargo run -p hythe-relay --example peer -- ws://127.0.0.1:7878 myroom src/main.rs Robin
```

## Limits

- Authentication is a shared secret, not per-user accounts, and nothing is
  encrypted without the TLS proxy above.
- If a seeder's first update never reaches the server and it reconnects after
  another person has already seeded the same file from different text, the
  two seeds can both appear. This needs a connection drop in the first moments
  of a brand new document.
- Saved documents are never deleted by the server; remove the files under the
  data directory to forget a room.
- A remote edit clears the receiving editor's undo history for that file.
- Only text files under the repository are shared.
