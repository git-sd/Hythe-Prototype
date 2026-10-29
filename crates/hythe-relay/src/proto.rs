//! Messages exchanged over the WebSocket, as JSON text frames.
//!
//! Clients join a *room* and open *documents* by name (the project-relative
//! path). The first client to open a document seeds it with its own text;
//! later ones adopt the shared state. After that, edits travel as CRDT
//! updates that the server relays to everyone else in the room.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 7878;

/// Someone in a room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: u64,
    pub name: String,
    /// 0xRRGGBB, used for their cursor.
    pub color: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Must be the first message.
    Hello { room: String, name: String, color: u32 },
    /// Ask for the shared state of `doc`.
    Open { doc: String },
    /// A CRDT update (base64) for `doc`.
    Update { doc: String, data: String },
    /// Cursor and selection anchor, as UTF-16 offsets into the document.
    Cursor { doc: String, head: u32, anchor: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent once after `Hello`: your id and who is already here.
    Welcome { you: u64, peers: Vec<Peer> },
    PeerJoined { peer: Peer },
    PeerLeft { id: u64 },
    /// Answer to `Open`. `data` is the full shared state, or `None` if nobody
    /// has the document yet; then `seed` says whether *you* should create it
    /// (the first asker) or wait for the seeder's update.
    State { doc: String, data: Option<String>, seed: bool },
    Update { doc: String, data: String, from: u64 },
    Cursor { doc: String, id: u64, head: u32, anchor: u32 },
    Error { message: String },
}

pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn unb64(text: &str) -> Result<Vec<u8>, String> {
    STANDARD.decode(text).map_err(|e| format!("bad base64: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_tagged_json() {
        let m = ClientMsg::Cursor { doc: "src/a.rs".into(), head: 3, anchor: 1 };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""t":"cursor""#), "{json}");
        assert_eq!(serde_json::from_str::<ClientMsg>(&json).unwrap(), m);

        let s = ServerMsg::State { doc: "x".into(), data: None, seed: true };
        assert_eq!(serde_json::from_str::<ServerMsg>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
        assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"nope"}"#).is_err());
    }

    #[test]
    fn base64_round_trips_binary() {
        let data = [0u8, 255, 1, 128, 7];
        assert_eq!(unb64(&b64(&data)).unwrap(), data);
        assert!(unb64("!!!").is_err());
    }
}
