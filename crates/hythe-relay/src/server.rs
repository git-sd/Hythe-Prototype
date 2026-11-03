//! The relay server. It keeps, per room, who is connected and the CRDT state of
//! every document opened so far, so a late joiner receives the current text.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

use crate::proto::{b64, unb64, ClientMsg, LockInfo, Peer, ServerMsg};
use crate::shared::SharedDoc;

type Tx = UnboundedSender<ServerMsg>;

struct DocState {
    doc: SharedDoc,
    /// The peer asked to create the document but hasn't sent its text yet.
    claimed_by: Option<u64>,
}

#[derive(Default)]
struct Room {
    peers: HashMap<u64, (Peer, Tx)>,
    docs: HashMap<String, DocState>,
    /// File locks: document name to the id of the peer holding it.
    locks: HashMap<String, u64>,
}

#[derive(Default)]
struct Hub {
    rooms: Mutex<HashMap<String, Room>>,
    next_id: AtomicU64,
}

impl Hub {
    fn join(&self, room: &str, name: String, color: u32, login: Option<String>, tx: Tx) -> (u64, Vec<Peer>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let peer = Peer { id, name, color, login };
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.entry(room.to_string()).or_default();
        let others: Vec<Peer> = room.peers.values().map(|(p, _)| p.clone()).collect();
        for (_, other_tx) in room.peers.values() {
            let _ = other_tx.send(ServerMsg::PeerJoined { peer: peer.clone() });
        }
        room.peers.insert(id, (peer, tx));
        (id, others)
    }

    fn leave(&self, room_name: &str, id: u64) {
        let mut rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get_mut(room_name) else { return };
        room.peers.remove(&id);
        let released = release_locks(room, id);
        for doc in room.docs.values_mut().filter(|d| d.claimed_by == Some(id)) {
            doc.claimed_by = None;
        }
        for (_, tx) in room.peers.values() {
            let _ = tx.send(ServerMsg::PeerLeft { id });
        }
        if released {
            broadcast_locks(room);
        }
        if room.peers.is_empty() {
            // Nobody left to remember the documents for.
            rooms.remove(room_name);
        }
    }

    fn open(&self, room_name: &str, id: u64, doc_name: String) -> ServerMsg {
        let mut rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get_mut(room_name) else {
            return ServerMsg::Error { message: "not in a room".into() };
        };
        let doc = room.docs.entry(doc_name.clone()).or_insert_with(|| DocState { doc: SharedDoc::new(), claimed_by: None });
        if !doc.doc.is_empty() {
            return ServerMsg::State { doc: doc_name, data: Some(b64(&doc.doc.state())), seed: false };
        }
        match doc.claimed_by {
            None => {
                doc.claimed_by = Some(id);
                ServerMsg::State { doc: doc_name, data: None, seed: true }
            }
            // Someone else is about to seed it; their update will reach this peer.
            Some(other) if other != id => ServerMsg::State { doc: doc_name, data: None, seed: false },
            Some(_) => ServerMsg::State { doc: doc_name, data: None, seed: true },
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
        let state = room.docs.entry(doc_name.clone()).or_insert_with(|| DocState { doc: SharedDoc::new(), claimed_by: None });
        if let Err(e) = state.doc.apply_update(&bytes) {
            return Some(ServerMsg::Error { message: e });
        }
        state.claimed_by = None;
        for (id, (_, tx)) in &room.peers {
            if *id != from {
                let _ = tx.send(ServerMsg::Update { doc: doc_name.clone(), data: data.clone(), from });
            }
        }
        None
    }

    fn cursor(&self, room_name: &str, from: u64, doc: String, head: u32, anchor: u32) {
        let rooms = self.rooms.lock().unwrap();
        let Some(room) = rooms.get(room_name) else { return };
        for (id, (_, tx)) in &room.peers {
            if *id != from {
                let _ = tx.send(ServerMsg::Cursor { doc: doc.clone(), id: from, head, anchor });
            }
        }
    }
}

// ---- File locks ----------------------------------------------------------

impl Hub {
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
            let _ = tx.send(ServerMsg::Locks { locks: lock_list(room) });
        }
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
        let _ = tx.send(ServerMsg::Locks { locks: locks.clone() });
    }
}

/// Drop every lock `id` holds. True if there were any.
fn release_locks(room: &mut Room, id: u64) -> bool {
    let before = room.locks.len();
    room.locks.retain(|_, owner| *owner != id);
    room.locks.len() != before
}

/// Listen on `addr` and serve forever.
pub async fn serve(addr: impl ToSocketAddrs) -> io::Result<()> {
    serve_listener(TcpListener::bind(addr).await?).await
}

/// Serve on an already-bound listener (lets callers pick port 0 and read it back).
pub async fn serve_listener(listener: TcpListener) -> io::Result<()> {
    let hub = Arc::new(Hub::default());
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(connection(hub.clone(), stream));
    }
}

async fn connection(hub: Arc<Hub>, stream: TcpStream) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else { return };
    let (mut sink, mut incoming) = ws.split();
    let (tx, mut outgoing) = unbounded_channel::<ServerMsg>();

    let writer = tokio::spawn(async move {
        while let Some(msg) = outgoing.recv().await {
            let Ok(json) = serde_json::to_string(&msg) else { continue };
            if sink.send(Message::text(json)).await.is_err() {
                break;
            }
        }
    });

    let mut session: Option<(String, u64)> = None;
    let mut seen_docs: HashSet<String> = HashSet::new();
    while let Some(Ok(frame)) = incoming.next().await {
        let text = match frame {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(msg) = serde_json::from_str::<ClientMsg>(text.as_str()) else {
            let _ = tx.send(ServerMsg::Error { message: "unrecognised message".into() });
            continue;
        };
        match (&session, msg) {
            (None, ClientMsg::Hello { room, name, color, login }) => {
                let (id, peers) = hub.join(&room, name, color, login, tx.clone());
                let _ = tx.send(ServerMsg::Welcome { you: id, peers });
                hub.send_locks(&room, id);
                session = Some((room, id));
            }
            (None, _) => {
                let _ = tx.send(ServerMsg::Error { message: "say hello first".into() });
            }
            (Some(_), ClientMsg::Hello { .. }) => {
                let _ = tx.send(ServerMsg::Error { message: "already joined".into() });
            }
            (Some((room, id)), ClientMsg::Open { doc }) => {
                seen_docs.insert(doc.clone());
                let _ = tx.send(hub.open(room, *id, doc));
            }
            (Some((room, id)), ClientMsg::Update { doc, data }) => {
                if let Some(err) = hub.update(room, *id, doc, data) {
                    let _ = tx.send(err);
                }
            }
            (Some((room, id)), ClientMsg::Lock { doc }) => {
                if let Some(denied) = hub.lock(room, *id, doc) {
                    let _ = tx.send(denied);
                }
            }
            (Some((room, id)), ClientMsg::Unlock { doc }) => hub.unlock(room, *id, &doc),
            (Some((room, id)), ClientMsg::Cursor { doc, head, anchor }) => hub.cursor(room, *id, doc, head, anchor),
        }
    }

    if let Some((room, id)) = session {
        hub.leave(&room, id);
    }
    writer.abort();
}
