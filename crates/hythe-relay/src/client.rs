//! A blocking-friendly client: the connection runs on its own thread and the
//! editor polls it, so the UI never waits on the network.
//!
//! The client reconnects by itself with exponential backoff. After every
//! (re)connection it says hello again and re-opens the documents it had opened;
//! the answers arrive as ordinary `State` messages for the caller to reconcile
//! (see [`crate::sync::DocSession`]). Edits sent while the link was down are
//! discarded here, because the resync sends the whole document state instead.

use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::proto::{close, ClientMsg, Peer, ServerMsg, DEFAULT_PORT};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The handshake finished (again, after a reconnect); you are `you`, with `peers` already present.
    Connected { you: u64, peers: Vec<Peer> },
    Message(ServerMsg),
    /// The link is down (or could not be made) and the client will try again.
    Reconnecting { attempt: u32, retry_in: Duration, reason: String },
    /// The client gave up for good: it was refused, or reconnecting is off.
    Disconnected(String),
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Shared secret for servers or rooms that require one.
    pub token: Option<String>,
    /// Retry after failures and dropped connections.
    pub reconnect: bool,
    pub connect_timeout: Duration,
    pub ping_interval: Duration,
    /// Treat the link as dead when the server has been silent this long.
    pub idle_timeout: Duration,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            token: None,
            reconnect: true,
            connect_timeout: Duration::from_secs(8),
            ping_interval: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(35),
            backoff_min: Duration::from_millis(500),
            backoff_max: Duration::from_secs(15),
        }
    }
}

pub struct Client {
    to_server: UnboundedSender<ClientMsg>,
    events: Receiver<Event>,
}

/// Accept what people type: `host`, `host:port`, `http(s)://…`, `ws(s)://…`.
/// Bare hosts become `ws://host:7878`.
pub fn normalize_url(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    if let Some(rest) = s.strip_prefix("https://") {
        return format!("wss://{rest}");
    }
    if let Some(rest) = s.strip_prefix("http://") {
        return format!("ws://{rest}");
    }
    if s.starts_with("ws://") || s.starts_with("wss://") {
        return s.to_string();
    }
    let has_port = s.rsplit_once(':').is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if has_port || s.contains('/') {
        format!("ws://{s}")
    } else {
        format!("ws://{s}:{DEFAULT_PORT}")
    }
}

/// Backoff before retry number `attempt` (1-based): doubling from `min` up to `max`, with up to 25% jitter.
pub fn backoff(opts: &Options, attempt: u32) -> Duration {
    let base = opts.backoff_min.saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
    let base = base.min(opts.backoff_max);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    base + base.mul_f64(0.25 * (nanos % 1000) as f64 / 1000.0)
}

/// Turn a low-level connection error into something a person can act on.
fn friendly(url: &str, e: &tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => {
            format!("nothing is listening at {url} (is hythe-relay running, and is the address right?)")
        }
        E::Io(io) => format!("could not reach {url}: {io}"),
        E::Url(_) => format!("`{url}` is not a valid server address (use ws://host:port or wss://host)"),
        E::Http(r) => format!("{url} answered HTTP {}; it does not look like a hythe-relay (check the address and port)", r.status()),
        E::HttpFormat(_) | E::Protocol(_) => format!("{url} is not speaking the relay protocol (wrong port?)"),
        E::Tls(t) => format!("secure connection to {url} failed: {t}"),
        other => format!("could not connect to {url}: {other}"),
    }
}

enum End {
    /// The `Client` was dropped.
    Dropped,
    /// Retrying will not help.
    Fatal(String),
    Lost { reason: String, was_connected: bool },
}

impl Client {
    /// One-shot connection: no retrying. Returns immediately; watch [`Client::poll`]
    /// for `Connected` or `Disconnected`.
    pub fn connect(url: &str, room: &str, name: &str, color: u32) -> Client {
        Client::connect_with(url, room, name, color, Options { reconnect: false, ..Options::default() })
    }

    /// Start connecting to `url` (for example `ws://127.0.0.1:7878`) and join `room`,
    /// reconnecting automatically according to `opts`.
    pub fn connect_with(url: &str, room: &str, name: &str, color: u32, opts: Options) -> Client {
        let (to_server, commands) = unbounded_channel::<ClientMsg>();
        let (event_tx, events) = channel::<Event>();
        let hello = ClientMsg::Hello { room: room.to_string(), name: name.to_string(), color, token: opts.token.clone() };
        let url = normalize_url(url);

        std::thread::spawn(move || {
            // wss:// needs a process-wide crypto provider; losing the race to another one is fine.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                let _ = event_tx.send(Event::Disconnected("could not start the network thread".into()));
                return;
            };
            runtime.block_on(run(url, hello, opts, commands, event_tx));
        });

        Client { to_server, events }
    }

    pub fn send(&self, msg: ClientMsg) {
        let _ = self.to_server.send(msg);
    }

    /// The next event from the server, if any.
    pub fn poll(&self) -> Option<Event> {
        match self.events.try_recv() {
            Ok(e) => Some(e),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

/// Note `Open`s (to replay later) and drop everything else queued while offline.
fn absorb(msg: ClientMsg, open_docs: &mut Vec<String>) {
    if let ClientMsg::Open { doc } = msg {
        if !open_docs.contains(&doc) {
            open_docs.push(doc);
        }
    }
}

async fn run(url: String, hello: ClientMsg, opts: Options, mut commands: UnboundedReceiver<ClientMsg>, events: Sender<Event>) {
    let mut open_docs: Vec<String> = Vec::new();
    let mut attempt = 0u32;
    loop {
        // Anything queued while we were away is stale: the resync sends full state.
        while let Ok(m) = commands.try_recv() {
            absorb(m, &mut open_docs);
        }
        match session(&url, &hello, &opts, &mut commands, &events, &mut open_docs).await {
            End::Dropped => return,
            End::Fatal(why) => {
                let _ = events.send(Event::Disconnected(why));
                return;
            }
            End::Lost { reason, was_connected } => {
                if !opts.reconnect {
                    let _ = events.send(Event::Disconnected(reason));
                    return;
                }
                attempt = if was_connected { 1 } else { attempt + 1 };
                let retry_in = backoff(&opts, attempt);
                if events.send(Event::Reconnecting { attempt, retry_in, reason }).is_err() {
                    return;
                }
                let wake = tokio::time::sleep(retry_in);
                tokio::pin!(wake);
                loop {
                    tokio::select! {
                        _ = &mut wake => break,
                        m = commands.recv() => match m {
                            Some(m) => absorb(m, &mut open_docs),
                            None => return,
                        },
                    }
                }
            }
        }
    }
}

async fn session(
    url: &str,
    hello: &ClientMsg,
    opts: &Options,
    commands: &mut UnboundedReceiver<ClientMsg>,
    events: &Sender<Event>,
    open_docs: &mut Vec<String>,
) -> End {
    let lost = |reason: String, was_connected: bool| End::Lost { reason, was_connected };
    let ws_cfg = WebSocketConfig::default().max_message_size(Some(64 << 20)).max_frame_size(Some(64 << 20));
    let connect = tokio::time::timeout(opts.connect_timeout, tokio_tungstenite::connect_async_with_config(url, Some(ws_cfg), false));
    let ws = match connect.await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return lost(friendly(url, &e), false),
        Err(_) => return lost(format!("timed out connecting to {url}"), false),
    };
    let (mut sink, mut incoming) = ws.split();

    // Hello first, then the documents we already had open.
    let mut handshake = vec![hello.clone()];
    handshake.extend(open_docs.iter().map(|d| ClientMsg::Open { doc: d.clone() }));
    for msg in handshake {
        let Ok(json) = serde_json::to_string(&msg) else { continue };
        if sink.send(Message::text(json)).await.is_err() {
            return lost("connection closed during the handshake".into(), false);
        }
    }

    let mut connected = false;
    let mut last_error: Option<String> = None;
    let mut last_seen = Instant::now();
    let mut ping = tokio::time::interval(opts.ping_interval.max(Duration::from_millis(20)));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = ping.tick().await; // the first tick is immediate

    loop {
        tokio::select! {
            frame = incoming.next() => {
                last_seen = Instant::now();
                match frame {
                    Some(Ok(Message::Text(t))) => {
                        let Ok(msg) = serde_json::from_str::<ServerMsg>(t.as_str()) else { continue };
                        let event = match msg {
                            ServerMsg::Welcome { you, peers } => {
                                connected = true;
                                Event::Connected { you, peers }
                            }
                            ServerMsg::Error { message } => {
                                last_error = Some(message.clone());
                                Event::Message(ServerMsg::Error { message })
                            }
                            other => Event::Message(other),
                        };
                        if events.send(event).is_err() {
                            return End::Dropped;
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        if sink.send(Message::Pong(p)).await.is_err() {
                            return lost("connection lost".into(), connected);
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let code = frame.as_ref().map(|f| u16::from(f.code));
                        let reason = frame.as_ref().map(|f| f.reason.to_string()).filter(|r| !r.is_empty());
                        let why = reason.or(last_error.take()).unwrap_or_else(|| "the server closed the connection".into());
                        return match code {
                            Some(close::UNAUTHORIZED) | Some(close::BAD_REQUEST) => End::Fatal(why),
                            _ => lost(why, connected),
                        };
                    }
                    None => return lost(last_error.unwrap_or_else(|| "the server closed the connection".into()), connected),
                    Some(Err(e)) => return lost(e.to_string(), connected),
                    Some(Ok(_)) => {}
                }
            }
            command = commands.recv() => match command {
                Some(msg) => {
                    if let ClientMsg::Open { doc } = &msg {
                        if !open_docs.contains(doc) {
                            open_docs.push(doc.clone());
                        }
                    }
                    let Ok(json) = serde_json::to_string(&msg) else { continue };
                    if sink.send(Message::text(json)).await.is_err() {
                        return lost("connection lost".into(), connected);
                    }
                }
                // The `Client` was dropped: say goodbye and stop.
                None => {
                    let _ = sink.send(Message::Close(None)).await;
                    return End::Dropped;
                }
            },
            _ = ping.tick() => {
                if last_seen.elapsed() > opts.idle_timeout {
                    return lost(format!("no answer from {url} for {}s", opts.idle_timeout.as_secs()), connected);
                }
                if sink.send(Message::Ping(Default::default())).await.is_err() {
                    return lost("connection lost".into(), connected);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalised() {
        assert_eq!(normalize_url("example.com"), "ws://example.com:7878");
        assert_eq!(normalize_url(" 10.0.0.5:9000 "), "ws://10.0.0.5:9000");
        assert_eq!(normalize_url("https://relay.example.com/"), "wss://relay.example.com");
        assert_eq!(normalize_url("http://localhost:1"), "ws://localhost:1");
        assert_eq!(normalize_url("wss://x.y/path"), "wss://x.y/path");
        assert_eq!(normalize_url("ws://127.0.0.1:7878"), "ws://127.0.0.1:7878");
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let o = Options::default();
        let d = |n| backoff(&o, n);
        assert!(d(1) >= Duration::from_millis(500) && d(1) <= Duration::from_millis(625));
        assert!(d(3) >= Duration::from_secs(2));
        assert!(d(30) <= Duration::from_millis(18_750) && d(30) >= Duration::from_secs(15));
    }
}
