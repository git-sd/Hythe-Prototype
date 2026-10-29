//! Terminal backend for running shells and CLI agents.
//!
//! Each [`Session`] pairs a child process on a pseudo-terminal with an
//! emulated screen. A dedicated I/O thread feeds output straight into the
//! emulator; the UI polls [`Session::take_dirty`] and pulls a [`Snapshot`]
//! only when something changed. [`TerminalManager`] holds many sessions, and
//! [`Profile`]s describe how to launch a shell or an agent such as Claude Code.
//!
//! The crate has no UI dependency: render a [`Snapshot`] however you like.

mod input;
mod manager;
mod palette;
mod profile;
mod session;
mod snapshot;

pub use input::{encode_key, encode_paste, Key, Mods};
pub use manager::{SessionInfo, TerminalManager};
pub use palette::{Palette, Rgb};
pub use profile::{available_agents, find_in_path, known_agents, Profile, ProfileKind, SpawnConfig};
pub use session::{Session, SessionId};
pub use snapshot::{style, Cell, Cursor, CursorKind, Snapshot};
