//! The relay server. It keeps, per room, who is connected and the CRDT state of
//! every document opened so far, so a late joiner receives the current text.
//!
//! Beyond relaying it: optionally persists documents to disk (`store`), checks
//! shared secrets (`auth`), keeps rooms in memory for a while after the last
//! peer leaves, detects dead connections with WebSocket pings, caps message
//! sizes and per-connection rates, and drops any client whose outgoing queue
//! overflows so one stuck connection can never stall the others.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::mpsc::{channel, error::TrySendError, Sender};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Bytes, Message};

use crate::auth::Auth;
use crate::proto::{b64, close, unb64, ClientMsg, LockInfo, Peer, ServerMsg};
use crate::shared::SharedDoc;
use crate::store::Store;

/// Protection against misbehaving or malicious clients.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Largest WebSocket message accepted (a whole document state travels in one).
    pub max_message_bytes: usize,
    pub max_peers_per_room: usize,
    pub max_docs_per_room: usize,
    /// Messages queued for one client before it is disconnected as too slow.
    pub max_queue: usize,
    /// Sustained messages per second a client may send, with a burst of twice that.
    pub max_msgs_per_sec: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_message_bytes: 16 << 20, max_peers_per_room: 64, max_docs_per_room: 2000, max_queue: 2048, max_msgs_per_sec: 200 }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Where to keep document snapshots; `None` keeps everything in memory only.
    pub data_dir: Option<PathBuf>,
    pub auth: Auth,
    pub limits: Limits,
    /// How long an empty room stays in memory after its last peer leaves.
    pub room_ttl: Duration,
    /// How often changed documents are written to `data_dir`.
    pub save_interval: Duration,
    /// How often an idle connection is pinged.
    pub ping_interval: Duration,
    /// A connection that sends nothing (not even a pong) for this long is dropped.
    pub idle_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            data_dir: None,
            auth: Auth::open(),
            limits: Limits::default(),
            room_ttl: Duration::from_secs(3600),
            save_interval: Duration::from_secs(1),
            ping_interval: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(45),
        }
    }
}

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ROOM_NAME: usize = 256;
const MAX_DOC_NAME: usize = 1024;
const MAX_PEER_NAME: usize = 64;

/// What the writer task puts on the wire.
enum Outgoing {
    Msg(ServerMsg),
    Ping,
    Pong(Bytes),
    Close(u16, String),
}

/// A bounded handle to one connection's outgoing queue.
#[derive(Clone)]
struct Tx {
    queue: Sender<Outgoing>,
    /// Wakes the connection task to tear the connection down.
    kick: Arc<Notify>,
}

impl Tx {
    fn push(&self, out: Outgoing) {
        if let Err(TrySendError::Full(_)) = self.queue.try_send(out) {
            // Too slow (or stuck): disconnect it rather than buffer without bound.
            self.kick.notify_one();
        }
    }

    fn send(&self, msg: ServerMsg) {
        self.push(Outgoing::Msg(msg));
    }

    /// Tell the client why, then close with an application close code.
    fn reject(&self, code: u16, message: &str) {
        self.send(ServerMsg::Error { message: message.to_string() });
        self.push(Outgoing::Close(code, message.to_string()));
    }
}

struct DocState {
    doc: SharedDoc,
    /// The document has content (or was deliberately created empty) in the room.
    seeded: bool,
    /// The peer asked to create the document but hasn't sent its text yet.
    claimed_by: Option<u64>,
    /// Peers told to wait for the seeder.
    waiting: Vec<u64>,
    /// Changed since last written to disk.
    dirty: bool,
}

impl DocState {
    fn new() -> DocState {
        DocState { doc: SharedDoc::new(), seeded: false, claimed_by: None, waiting: Vec::new(), dirty: false }
    }
}

#[derive(Default)]
struct Room {
    peers: HashMap<u64, (Peer, Tx)>,
    docs: HashMap<String, DocState>,
    empty_since: Option<Instant>,
    /// File locks: document name to the id of the peer holding it. Never persisted;
    /// a lock dies with its holder's connection.
    locks: HashMap<String, u64>,
}

impl Room {
    /// Give up `id`'s claims and waits; hand each abandoned seed to the next waiter.
    fn release(&mut self, id: u64) {
        let Room { peers, docs, .. } = self;
        for (name, d) in docs.iter_mut() {
            d.waiting.retain(|w| *w != id);
            if d.claimed_by != Some(id) {
                continue;
            }
            d.claimed_by = None;
            if d.seeded {
                continue;
            }
            while !d.waiting.is_empty() {
                let next = d.waiting.remove(0);
                if let Some((_, tx)) = peers.get(&next) {
                    d.claimed_by = Some(next);
                    tx.send(ServerMsg::State { doc: name.clone(), data: None, seed: true });
                    break;
                }
            }
        }
    }
}

struct Hub {
    rooms: Mutex<HashMap<String, Room>>,
    next_id: AtomicU64,
    cfg: ServerConfig,
    store: Option<Store>,
}

impl Hub {
    fn join(&self, room_name: &str, name: String, color: u32, login: Option<String>, tx: Tx) -> Result<(u64, Vec<Peer>), String> {
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.entry(room_name.to_string()).or_default();
        if room.peers.len() >= self.cfg.limits.max_peers_per_room {
            return Err("this room is full".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let peer = Peer { id, name, color, login };
        let others: Vec<Peer> = room.peers.values().map(|(p, _)| p.clone()).collect();
        for (_, other_tx) in room.peers.values() {
            other_tx.send(ServerMsg::PeerJoined { peer: peer.clone() });
        }
        room.peers.insert(id, (peer, tx));
        room.empty_since = None;
        Ok((id, others))
    }

    fn leave(&self, room_name: &str, id: u64) {
        let mut rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get_mut(room_name) else { return };
        room.peers.remove(&id);
        room.release(id);
        let released = release_locks(room, id);
        for (_, tx) in room.peers.values() {
            tx.send(ServerMsg::PeerLeft { id });
        }
        if released {
            broadcast_locks(room);
        }
        if room.peers.is_empty() {
            // Keep the documents for a while so rejoining peers find them (see `sweep`).
            room.empty_since = Some(Instant::now());
        }
    }

    fn open(&self, room_name: &str, id: u64, doc_name: String) -> ServerMsg {
        let mut rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get_mut(room_name) else {
            return ServerMsg::Error { message: "not in a room".into() };
        };
        if !room.docs.contains_key(&doc_name) && room.docs.len() >= self.cfg.limits.max_docs_per_room {
            return ServerMsg::Error { message: "too many documents in this room".into() };
        }
        let d = room.docs.entry(doc_name.clone()).or_insert_with(DocState::new);
        d.waiting.retain(|w| *w != id);
        if d.seeded {
            return ServerMsg::State { doc: doc_name, data: Some(b64(&d.doc.state())), seed: false };
        }
        match d.claimed_by {
            None => {
                d.claimed_by = Some(id);
                ServerMsg::State { doc: doc_name, data: None, seed: true }
            }
            Some(holder) if holder == id => ServerMsg::State { doc: doc_name, data: None, seed: true },
            // Someone else is about to seed it; their update will reach this peer.
            Some(_) => {
                d.waiting.push(id);
                ServerMsg::State { doc: doc_name, data: None, seed: false }
            }
        }
    }

    fn update(&self, room_name: &str, from: u64, doc_name: String, data: String) -> Option<ServerMsg> {
        let bytes = match unb64(&data) {
            Ok(b) => b,
            Err(e) => return Some(ServerMsg::Error { message: e }),
        };
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.get_mut(room_name)?;
        if let Some(denied) = lock_denial(room, from, &doc_name) {
            return Some(denied);
        }
        if !room.docs.contains_key(&doc_name) && room.docs.len() >= self.cfg.limits.max_docs_per_room {
            return Some(ServerMsg::Error { message: "too many documents in this room".into() });
        }
        let state = room.docs.entry(doc_name.clone()).or_insert_with(DocState::new);
        if let Err(e) = state.doc.apply_update(&bytes) {
            return Some(ServerMsg::Error { message: e });
        }
        state.seeded = true;
        state.claimed_by = None;
        state.waiting.clear();
        state.dirty = true;
        for (id, (_, tx)) in &room.peers {
            if *id != from {
                tx.send(ServerMsg::Update { doc: doc_name.clone(), data: data.clone(), from });
            }
        }
        None
    }

    fn cursor(&self, room_name: &str, from: u64, doc: String, head: u32, anchor: u32) {
        let rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get(room_name) else { return };
        for (id, (_, tx)) in &room.peers {
            if *id != from {
                tx.send(ServerMsg::Cursor { doc: doc.clone(), id: from, head, anchor });
            }
        }
    }

    /// `id` asks for the lock on `doc`. Returns `LockDenied` if someone else has it.
    fn lock(&self, room_name: &str, id: u64, doc: String) -> Option<ServerMsg> {
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.get_mut(room_name)?;
        if let Some(denied) = lock_denial(room, id, &doc) {
            return Some(denied);
        }
        if room.locks.insert(doc, id).is_none() {
            broadcast_locks(room);
        }
        None
    }

    /// Only the holder can release; anyone else is ignored.
    fn unlock(&self, room_name: &str, id: u64, doc: &str) {
        let mut rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get_mut(room_name) else { return };
        if room.locks.get(doc) == Some(&id) {
            room.locks.remove(doc);
            broadcast_locks(room);
        }
    }

    /// Tell one peer (just welcomed) about the current locks.
    fn send_locks(&self, room_name: &str, id: u64) {
        let rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get(room_name) else { return };
        if let Some((_, tx)) = room.peers.get(&id) {
            tx.send(ServerMsg::Locks { locks: lock_list(room) });
        }
    }

    /// Bring a room's saved documents into memory the first time it is needed.
    async fn ensure_loaded(self: &Arc<Self>, room_name: &str) {
        let Some(store) = self.store.clone() else { return };
        if self.rooms.lock().unwrap().contains_key(room_name) {
            return;
        }
        let name = room_name.to_string();
        let saved = tokio::task::spawn_blocking(move || store.load_room(&name)).await.unwrap_or_default();
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.entry(room_name.to_string()).or_default();
        for (doc_name, bytes) in saved {
            if room.docs.contains_key(&doc_name) {
                continue;
            }
            let mut d = DocState::new();
            if d.doc.apply_update(&bytes).is_ok() {
                d.seeded = true;
                room.docs.insert(doc_name, d);
            } else {
                eprintln!("hythe-relay: skipping unreadable saved document {doc_name:?} in room {room_name:?}");
            }
        }
    }

    /// Snapshots of every changed document, marking them clean.
    fn take_dirty(&self) -> Vec<(String, String, Vec<u8>)> {
        let mut rooms = self.rooms.lock().unwrap();
        let mut out = Vec::new();
        for (room_name, room) in rooms.iter_mut() {
            for (doc_name, d) in room.docs.iter_mut().filter(|(_, d)| d.dirty) {
                d.dirty = false;
                out.push((room_name.clone(), doc_name.clone(), d.doc.state()));
            }
        }
        out
    }

    fn mark_dirty(&self, room: &str, doc: &str) {
        if let Some(d) = self.rooms.lock().unwrap().get_mut(room).and_then(|r| r.docs.get_mut(doc)) {
            d.dirty = true;
        }
    }

    async fn flush(self: &Arc<Self>) {
        let Some(store) = self.store.clone() else { return };
        let items = self.take_dirty();
        if items.is_empty() {
            return;
        }
        let failed = tokio::task::spawn_blocking(move || {
            items
                .into_iter()
                .filter_map(|(room, doc, bytes)| match store.save(&room, &doc, &bytes) {
                    Ok(()) => None,
                    Err(e) => {
                        eprintln!("hythe-relay: could not save {doc:?} in {room:?}: {e}");
                        Some((room, doc))
                    }
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        for (room, doc) in failed {
            self.mark_dirty(&room, &doc); // try again next time
        }
    }

    /// Forget rooms that have been empty longer than `room_ttl` and are safely on disk.
    fn sweep(&self) {
        let ttl = self.cfg.room_ttl;
        self.rooms.lock().unwrap().retain(|_, room| match room.empty_since {
            Some(t) if room.peers.is_empty() && t.elapsed() >= ttl => self.store.is_some() && room.docs.values().any(|d| d.dirty),
            _ => true,
        });
    }
}

/// `LockDenied` if `doc` is locked by someone other than `id`.
fn lock_denial(room: &Room, id: u64, doc: &str) -> Option<ServerMsg> {
    let owner = *room.locks.get(doc)?;
    if owner == id {
        return None;
    }
    let owner_name = room.peers.get(&owner).map(|(p, _)| p.name.clone()).unwrap_or_default();
    Some(ServerMsg::LockDenied { doc: doc.to_string(), owner_name })
}

fn lock_list(room: &Room) -> Vec<LockInfo> {
    let mut list: Vec<LockInfo> = room
        .locks
        .iter()
        .filter_map(|(doc, owner)| {
            let (p, _) = room.peers.get(owner)?;
            Some(LockInfo { doc: doc.clone(), owner_id: *owner, owner_name: p.name.clone(), login: p.login.clone() })
        })
        .collect();
    list.sort_by(|a, b| a.doc.cmp(&b.doc));
    list
}

fn broadcast_locks(room: &Room) {
    let locks = lock_list(room);
    for (_, tx) in room.peers.values() {
        tx.send(ServerMsg::Locks { locks: locks.clone() });
    }
}

/// Drop every lock `id` holds. True if there were any.
fn release_locks(room: &mut Room, id: u64) -> bool {
    let before = room.locks.len();
    room.locks.retain(|_, owner| *owner != id);
    room.locks.len() != before
}

/// A running relay: shared state plus the entry points to serve it.
#[derive(Clone)]
pub struct Relay {
    hub: Arc<Hub>,
}

impl Relay {
    pub fn new(cfg: ServerConfig) -> io::Result<Relay> {
        let store = cfg.data_dir.as_ref().map(Store::open).transpose()?;
        Ok(Relay { hub: Arc::new(Hub { rooms: Mutex::new(HashMap::new()), next_id: AtomicU64::new(0), cfg, store }) })
    }

    /// Write every changed document to disk now (call before shutting down).
    pub async fn flush(&self) {
        self.hub.flush().await;
    }

    /// Accept connections forever.
    pub async fn serve(&self, listener: TcpListener) -> io::Result<()> {
        let saver = {
            let hub = self.hub.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(hub.cfg.save_interval.max(Duration::from_millis(50)));
                loop {
                    tick.tick().await;
                    hub.flush().await;
                    hub.sweep();
                }
            })
        };
        let result = async {
            loop {
                let (stream, _) = listener.accept().await?;
                tokio::spawn(connection(self.hub.clone(), stream));
            }
        }
        .await;
        saver.abort();
        result
    }
}

/// Listen on `addr` and serve forever (open, in-memory).
pub async fn serve(addr: impl ToSocketAddrs) -> io::Result<()> {
    serve_listener(TcpListener::bind(addr).await?).await
}

/// Serve on an already-bound listener (lets callers pick port 0 and read it back).
pub async fn serve_listener(listener: TcpListener) -> io::Result<()> {
    serve_with(listener, ServerConfig::default()).await
}

/// Serve with persistence, auth and limits from `config`.
pub async fn serve_with(listener: TcpListener, config: ServerConfig) -> io::Result<()> {
    Relay::new(config)?.serve(listener).await
}

/// Leaky bucket for inbound messages.
struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    last: Instant,
}

impl Bucket {
    fn new(per_sec: u32) -> Bucket {
        let rate = per_sec.max(1) as f64;
        Bucket { tokens: rate * 2.0, rate, burst: rate * 2.0, last: Instant::now() }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f64() * self.rate).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn valid_name(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

async fn connection(hub: Arc<Hub>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let limits = &hub.cfg.limits;
    let ws_cfg = WebSocketConfig::default().max_message_size(Some(limits.max_message_bytes)).max_frame_size(Some(limits.max_message_bytes));
    let accepted = tokio::time::timeout(HELLO_TIMEOUT, tokio_tungstenite::accept_async_with_config(stream, Some(ws_cfg))).await;
    let Ok(Ok(ws)) = accepted else { return };
    let (mut sink, mut incoming) = ws.split();
    let (queue, mut outgoing) = channel::<Outgoing>(limits.max_queue.max(8));
    let kick = Arc::new(Notify::new());
    let tx = Tx { queue, kick: kick.clone() };

    let writer = {
        let kick = kick.clone();
        tokio::spawn(async move {
            while let Some(out) = outgoing.recv().await {
                let (frame, last) = match out {
                    Outgoing::Msg(m) => match serde_json::to_string(&m) {
                        Ok(json) => (Message::text(json), false),
                        Err(_) => continue,
                    },
                    Outgoing::Ping => (Message::Ping(Bytes::new()), false),
                    Outgoing::Pong(p) => (Message::Pong(p), false),
                    Outgoing::Close(code, reason) => {
                        (Message::Close(Some(CloseFrame { code: CloseCode::from(code), reason: reason.into() })), true)
                    }
                };
                match tokio::time::timeout(WRITE_TIMEOUT, sink.send(frame)).await {
                    Ok(Ok(())) => {}
                    _ => break,
                }
                if last {
                    break;
                }
            }
            kick.notify_one();
        })
    };

    let mut session: Option<(String, u64)> = None;
    let mut bucket = Bucket::new(limits.max_msgs_per_sec);
    let started = Instant::now();
    let mut last_seen = Instant::now();
    let mut last_ping = Instant::now();
    let tick_every = (hub.cfg.ping_interval).min(hub.cfg.idle_timeout / 4).min(Duration::from_secs(1)).max(Duration::from_millis(20));
    let mut tick = tokio::time::interval(tick_every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = kick.notified() => break,
            _ = tick.tick() => {
                if last_seen.elapsed() > hub.cfg.idle_timeout || (session.is_none() && started.elapsed() > HELLO_TIMEOUT) {
                    break;
                }
                if last_ping.elapsed() >= hub.cfg.ping_interval {
                    last_ping = Instant::now();
                    tx.push(Outgoing::Ping);
                }
            }
            frame = incoming.next() => {
                last_seen = Instant::now();
                let text = match frame {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Ping(p))) => { tx.push(Outgoing::Pong(p)); continue; }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                if !bucket.take() {
                    tx.reject(close::LIMIT, "sending too fast; slow down");
                    break_after_flush().await;
                    break;
                }
                let Ok(msg) = serde_json::from_str::<ClientMsg>(text.as_str()) else {
                    tx.send(ServerMsg::Error { message: "unrecognised message".into() });
                    continue;
                };
                match (&session, msg) {
                    (None, ClientMsg::Hello { room, name, color, token, login }) => {
                        if !valid_name(&room, MAX_ROOM_NAME) || !valid_name(&name, MAX_PEER_NAME) {
                            tx.reject(close::BAD_REQUEST, "invalid room or name");
                            break_after_flush().await;
                            break;
                        }
                        if let Err(why) = hub.cfg.auth.check(&room, token.as_deref()) {
                            // Slow down guessing.
                            tokio::time::sleep(Duration::from_millis(250)).await;
                            tx.reject(close::UNAUTHORIZED, &why);
                            break_after_flush().await;
                            break;
                        }
                        hub.ensure_loaded(&room).await;
                        match hub.join(&room, name, color, login, tx.clone()) {
                            Ok((id, peers)) => {
                                tx.send(ServerMsg::Welcome { you: id, peers });
                                hub.send_locks(&room, id);
                                session = Some((room, id));
                            }
                            Err(why) => {
                                tx.reject(close::LIMIT, &why);
                                break_after_flush().await;
                                break;
                            }
                        }
                    }
                    (None, _) => tx.send(ServerMsg::Error { message: "say hello first".into() }),
                    (Some(_), ClientMsg::Hello { .. }) => tx.send(ServerMsg::Error { message: "already joined".into() }),
                    (Some((room, id)), ClientMsg::Open { doc }) => {
                        if !valid_name(&doc, MAX_DOC_NAME) {
                            tx.send(ServerMsg::Error { message: "invalid document name".into() });
                        } else {
                            tx.send(hub.open(room, *id, doc));
                        }
                    }
                    (Some((room, id)), ClientMsg::Update { doc, data }) => {
                        if !valid_name(&doc, MAX_DOC_NAME) {
                            tx.send(ServerMsg::Error { message: "invalid document name".into() });
                        } else if let Some(err) = hub.update(room, *id, doc, data) {
                            tx.send(err);
                        }
                    }
                    (Some((room, id)), ClientMsg::Lock { doc }) => {
                        if !valid_name(&doc, MAX_DOC_NAME) {
                            tx.send(ServerMsg::Error { message: "invalid document name".into() });
                        } else if let Some(denied) = hub.lock(room, *id, doc) {
                            tx.send(denied);
                        }
                    }
                    (Some((room, id)), ClientMsg::Unlock { doc }) => hub.unlock(room, *id, &doc),
                    (Some((room, id)), ClientMsg::Cursor { doc, head, anchor }) => hub.cursor(room, *id, doc, head, anchor),
                }
            }
        }
    }

    if let Some((room, id)) = session {
        hub.leave(&room, id);
    }
    writer.abort();
}

/// Give the writer a moment to put a rejection on the wire before the socket is dropped.
async fn break_after_flush() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}
