//! The client side of the open/seed/adopt protocol, including resync after a
//! reconnect, as a small state machine that does no I/O. The editor and the
//! tests both drive it.
//!
//! The rules:
//! - A fresh document that gets `State { data: Some }` adopts the room's text.
//! - A fresh document that gets `seed: true` becomes the room's text (it sends
//!   its local text); with `seed: false` and no data it waits for the seeder's
//!   update, which it then adopts.
//! - A document that was already in sync keeps its CRDT history across a
//!   reconnect. Edits made while offline sit in that history; on the next
//!   `State` it merges the server's state and sends its whole state back.
//!   Applying an update twice is a no-op in a CRDT, so nothing is duplicated.

use crate::shared::{SharedDoc, TextChange};

/// What the editor must do to its plain-text copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remote {
    Nothing,
    /// Throw the local text away and use this.
    Replace(String),
    /// Apply these edits to the local text.
    Patch(Vec<TextChange>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// A CRDT update to send to the server as `ClientMsg::Update`.
    pub send: Option<Vec<u8>>,
    pub remote: Remote,
}

impl Outcome {
    fn nothing() -> Outcome {
        Outcome { send: None, remote: Remote::Nothing }
    }
}

#[derive(Default)]
pub struct DocSession {
    pub doc: SharedDoc,
    /// Exchanged state with the room (seeded it, or adopted its text).
    pub synced: bool,
    /// Reconnected (or about to) and not yet reconciled with the server: local
    /// edits go into the CRDT but are only sent with the next full-state push.
    resyncing: bool,
}

impl DocSession {
    pub fn new() -> DocSession {
        DocSession::default()
    }

    pub fn is_resyncing(&self) -> bool {
        self.resyncing
    }

    /// The connection dropped. History is kept; sending resumes after the next `State`.
    pub fn on_disconnect(&mut self) {
        if self.synced {
            self.resyncing = true;
        }
    }

    /// A local edit. Returns the update to send, or `None` while not in sync
    /// with the room (the edit is still recorded and goes out with the resync).
    pub fn local_edit(&mut self, pos: u32, remove: u32, insert: &str) -> Option<Vec<u8>> {
        if !self.synced {
            return None;
        }
        let update = self.doc.replace(pos, remove, insert);
        (!self.resyncing).then_some(update)
    }

    /// The server answered an `Open`. `local_text` is the editor's current text.
    pub fn on_state(&mut self, data: Option<&[u8]>, seed: bool, local_text: &str) -> Result<Outcome, String> {
        match (data, self.synced) {
            (Some(bytes), false) => {
                self.doc.apply_update(bytes)?;
                self.synced = true;
                self.resyncing = false;
                Ok(Outcome { send: None, remote: Remote::Replace(self.doc.text()) })
            }
            (Some(bytes), true) => {
                let changes = self.doc.apply_update(bytes)?;
                self.resyncing = false;
                Ok(Outcome { send: Some(self.doc.state()), remote: Remote::Patch(changes) })
            }
            (None, false) if seed => {
                let update = self.doc.replace(0, 0, local_text);
                self.synced = true;
                self.resyncing = false;
                Ok(Outcome { send: Some(update), remote: Remote::Nothing })
            }
            (None, true) if seed => {
                // The server lost the document (or never got our seed); we are the source.
                self.resyncing = false;
                Ok(Outcome { send: Some(self.doc.state()), remote: Remote::Nothing })
            }
            // Someone else is seeding; their update arrives next.
            (None, _) => Ok(Outcome::nothing()),
        }
    }

    /// A peer's update arrived.
    pub fn on_update(&mut self, bytes: &[u8]) -> Result<Outcome, String> {
        let was_synced = self.synced;
        let was_resyncing = self.resyncing;
        let changes = self.doc.apply_update(bytes)?;
        self.synced = true;
        if !was_synced {
            return Ok(Outcome { send: None, remote: Remote::Replace(self.doc.text()) });
        }
        // Reconnected and the room is alive again: push what we did offline.
        let send = was_resyncing.then(|| self.doc.state());
        self.resyncing = false;
        Ok(Outcome { send, remote: Remote::Patch(changes) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::apply_changes;

    fn adopt(session: &mut DocSession, mirror: &mut String, o: &Outcome) {
        match &o.remote {
            Remote::Nothing => {}
            Remote::Replace(t) => *mirror = t.clone(),
            Remote::Patch(c) => *mirror = apply_changes(mirror, c),
        }
        assert_eq!(*mirror, session.doc.text());
    }

    #[test]
    fn offline_edits_merge_without_duplication() {
        // Room seeded by A; B adopts.
        let mut a = DocSession::new();
        let seed = a.on_state(None, true, "hello world").unwrap().send.unwrap();
        let mut b = DocSession::new();
        let mut b_text = String::new();
        let o = b.on_update(&seed).unwrap();
        adopt(&mut b, &mut b_text, &o);

        // Both go offline and edit.
        a.on_disconnect();
        b.on_disconnect();
        assert!(a.local_edit(0, 0, "A:").is_none());
        assert!(b.local_edit(11, 0, "!").is_none());
        let mut b_local = b_text.clone();
        b_local.push('!');

        // Server (kept memory) has the seed. A reconnects first.
        let mut server = DocSession::new();
        server.doc.apply_update(&seed).unwrap();
        let o = a.on_state(Some(&server.doc.state()), false, "").unwrap();
        server.doc.apply_update(&o.send.unwrap()).unwrap();
        // B reconnects.
        let o = b.on_state(Some(&server.doc.state()), false, &b_local).unwrap();
        let mut mirror = b_local;
        adopt(&mut b, &mut mirror, &o);
        server.doc.apply_update(&o.send.unwrap()).unwrap();
        // A learns B's edits; replaying everything again changes nothing.
        let ob = a.on_update(&server.doc.state()).unwrap();
        let _ = ob;
        for _ in 0..2 {
            a.doc.apply_update(&server.doc.state()).unwrap();
            b.doc.apply_update(&server.doc.state()).unwrap();
        }
        assert_eq!(a.doc.text(), "A:hello world!");
        assert_eq!(b.doc.text(), "A:hello world!");
        assert_eq!(mirror, "A:hello world!");
    }

    #[test]
    fn a_synced_session_repopulates_a_server_that_lost_the_document() {
        let mut a = DocSession::new();
        a.on_state(None, true, "keep me").unwrap();
        a.on_disconnect();
        let o = a.on_state(None, true, "keep me").unwrap();
        let mut server = SharedDoc::new();
        server.apply_update(&o.send.unwrap()).unwrap();
        assert_eq!(server.text(), "keep me");
        assert!(!a.is_resyncing());
    }

    #[test]
    fn waiter_with_history_pushes_offline_edits_when_the_room_returns() {
        let mut a = DocSession::new();
        let seed = a.on_state(None, true, "abc").unwrap().send.unwrap();
        a.on_disconnect();
        a.local_edit(3, 0, "d");
        // Someone else holds the claim: nothing to send yet, still resyncing.
        let o = a.on_state(None, false, "abcd").unwrap();
        assert!(o.send.is_none() && a.is_resyncing());
        // The seeder (same lineage here) shows up.
        let o = a.on_update(&seed).unwrap();
        assert!(o.send.is_some() && !a.is_resyncing());
    }

    #[test]
    fn fresh_waiter_adopts_the_seeders_text() {
        let mut waiter = DocSession::new();
        assert_eq!(waiter.on_state(None, false, "stale local").unwrap(), Outcome::nothing());
        let mut seeder = DocSession::new();
        let u = seeder.on_state(None, true, "").unwrap().send.unwrap();
        // Even an empty seed unblocks the waiter.
        let o = waiter.on_update(&u).unwrap();
        assert_eq!(o.remote, Remote::Replace(String::new()));
        assert!(waiter.synced);
    }
}
