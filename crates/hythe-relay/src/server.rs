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

use crate::proto::{b64, unb64, ClientMsg, Peer, ServerMsg};
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
}

#[derive(Default)]
struct Hub {
    rooms: Mutex<HashMap<String, Room>>,
    next_id: AtomicU64,
}

impl Hub {
    fn join(&self, room: &str, name: String, color: u32, tx: Tx) -> (u64, Vec<Peer>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let peer = Peer { id, name, color };
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
        for doc in room.docs.values_mut().filter(|d| d.claimed_by == Some(id)) {
            doc.claimed_by = None;
        }
        for (_, tx) in room.peers.values() {
            let _ = tx.send(ServerMsg::PeerLeft { id });
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
            (None, ClientMsg::Hello { room, name, color }) => {
                let (id, peers) = hub.join(&room, name, color, tx.clone());
                let _ = tx.send(ServerMsg::Welcome { you: id, peers });
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
            (Some((room, id)), ClientMsg::Cursor { doc, head, anchor }) => hub.cursor(room, *id, doc, head, anchor),
        }
    }

    if let Some((room, id)) = session {
        hub.leave(&room, id);
    }
    writer.abort();
}
