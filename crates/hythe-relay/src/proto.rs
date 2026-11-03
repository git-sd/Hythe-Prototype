//! Messages exchanged over the WebSocket, as JSON text frames.
//!
//! Clients join a *room* and open *documents* by name (the project-relative
//! path). The first client to open a document seeds it with its own text;
//! later ones adopt the shared state. After that, edits travel as CRDT
//! updates that the server relays to everyone else in the room.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 7878;

/// WebSocket close codes (application range) the server uses, so a client can
/// tell "don't retry" from "network trouble".
pub mod close {
    /// Malformed handshake or invalid names.
    pub const BAD_REQUEST: u16 = 4400;
    /// Missing or wrong token.
    pub const UNAUTHORIZED: u16 = 4401;
    /// Room is full, or the client is too fast or too slow.
    pub const LIMIT: u16 = 4429;
}

/// Someone in a room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: u64,
    pub name: String,
    /// 0xRRGGBB, used for their cursor.
    pub color: u32,
    /// GitHub account, when the person's app knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

/// A file somebody has locked: only they may edit it until they unlock it or leave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    pub doc: String,
    pub owner_id: u64,
    pub owner_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Must be the first message.
    Hello {
        room: String,
        name: String,
        color: u32,
        /// Shared secret, when the server or room requires one. Absent for open servers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        login: Option<String>,
    },
    /// Ask for the shared state of `doc`.
    Open { doc: String },
    /// A CRDT update (base64) for `doc`.
    Update { doc: String, data: String },
    /// Cursor and selection anchor, as UTF-16 offsets into the document.
    Cursor { doc: String, head: u32, anchor: u32 },
    /// Claim `doc` so nobody else can edit it. First come, first served.
    Lock { doc: String },
    /// Release a lock you hold.
    Unlock { doc: String },
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
    /// Every lock in the room; sent after `Welcome` and whenever locks change.
    Locks { locks: Vec<LockInfo> },
    /// Your `Lock` or `Update` for `doc` was refused: `owner_name` holds the lock.
    LockDenied { doc: String, owner_name: String },
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
    fn hello_without_a_token_still_parses_and_omits_it() {
        let old = r#"{"t":"hello","room":"r","name":"n","color":1}"#;
        let m: ClientMsg = serde_json::from_str(old).unwrap();
        assert_eq!(m, ClientMsg::Hello { room: "r".into(), name: "n".into(), color: 1, token: None, login: None });
        assert!(!serde_json::to_string(&m).unwrap().contains("token"));
        let with = ClientMsg::Hello { room: "r".into(), name: "n".into(), color: 1, token: Some("s".into()), login: Some("octo".into()) };
        assert_eq!(serde_json::from_str::<ClientMsg>(&serde_json::to_string(&with).unwrap()).unwrap(), with);
    }

    #[test]
    fn old_peer_without_login_still_parses() {
        let p: Peer = serde_json::from_str(r#"{"id":1,"name":"n","color":1}"#).unwrap();
        assert_eq!(p.login, None);
    }

    #[test]
    fn base64_round_trips_binary() {
        let data = [0u8, 255, 1, 128, 7];
        assert_eq!(unb64(&b64(&data)).unwrap(), data);
        assert!(unb64("!!!").is_err());
    }
}
