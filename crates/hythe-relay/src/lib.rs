//! Real-time collaboration relay: a small WebSocket server that forwards CRDT
//! updates between editors in the same room, plus a client and the shared
//! document type they edit. Self-hostable, with no third-party service.

pub mod client;
pub mod proto;
pub mod server;
pub mod shared;

pub use client::{Client, Event};
pub use proto::{ClientMsg, Peer, ServerMsg, DEFAULT_PORT};
pub use shared::{apply_changes, utf16_len, SharedDoc, TextChange};
