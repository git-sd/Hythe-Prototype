//! A shared text document backed by a CRDT (`yrs`).
//!
//! All positions are UTF-16 code-unit offsets, which is what `yrs` counts in.
//! Editors that index by `char` convert at the boundary.

use std::sync::{Arc, Mutex};

use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, GetString, Observable, OffsetKind, Options, Out, ReadTxn, StateVector, Text, TextRef, Transact, Update};
use yrs::types::Delta;

/// One edit to apply, in order, to a plain-text copy of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChange {
    /// UTF-16 offset where the change happens (in the text as changed by the earlier ones).
    pub pos: u32,
    /// UTF-16 units to delete at `pos`.
    pub remove: u32,
    pub insert: String,
}

pub fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

pub struct SharedDoc {
    doc: Doc,
    text: TextRef,
    /// Changes the CRDT reported since the last drain.
    changes: Arc<Mutex<Vec<TextChange>>>,
    _subscription: yrs::Subscription,
}

impl Default for SharedDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedDoc {
    pub fn new() -> SharedDoc {
        // UTF-16 offsets, so positions match what editors convert to and from chars.
        let doc = Doc::with_options(Options { offset_kind: OffsetKind::Utf16, ..Options::default() });
        let text = doc.get_or_insert_text("text");
        let changes: Arc<Mutex<Vec<TextChange>>> = Arc::default();
        let sink = changes.clone();
        let subscription = text.observe(move |txn, event| {
            let mut pos = 0u32;
            let mut out = sink.lock().unwrap();
            for delta in event.delta(txn) {
                match delta {
                    Delta::Retain(n, _) => pos += n,
                    Delta::Inserted(value, _) => {
                        let s = match value {
                            Out::Any(Any::String(s)) => s.to_string(),
                            other => other.to_string(),
                        };
                        let len = utf16_len(&s);
                        out.push(TextChange { pos, remove: 0, insert: s });
                        pos += len;
                    }
                    Delta::Deleted(n) => out.push(TextChange { pos, remove: *n, insert: String::new() }),
                }
            }
        });
        SharedDoc { doc, text, changes, _subscription: subscription }
    }

    pub fn text(&self) -> String {
        self.text.get_string(&self.doc.transact())
    }

    /// Length in UTF-16 units.
    pub fn len(&self) -> u32 {
        self.text.len(&self.doc.transact())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The whole document as one update, for a peer that has nothing.
    pub fn state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    /// Replace `remove` units at `pos` with `insert`; returns the update to send.
    pub fn replace(&mut self, pos: u32, remove: u32, insert: &str) -> Vec<u8> {
        use yrs::ReadTxn as _;
        let before = self.doc.transact().state_vector();
        {
            let mut txn = self.doc.transact_mut();
            if remove > 0 {
                self.text.remove_range(&mut txn, pos, remove);
            }
            if !insert.is_empty() {
                self.text.insert(&mut txn, pos, insert);
            }
        }
        // Our own edit also fires the observer; those changes are already in the editor.
        self.changes.lock().unwrap().clear();
        self.doc.transact().encode_state_as_update_v1(&before)
    }

    /// Apply a remote update; returns the edits to mirror into a plain-text copy.
    pub fn apply_update(&mut self, bytes: &[u8]) -> Result<Vec<TextChange>, String> {
        let update = Update::decode_v1(bytes).map_err(|e| format!("bad update: {e}"))?;
        self.changes.lock().unwrap().clear();
        {
            let mut txn = self.doc.transact_mut();
            txn.apply_update(update).map_err(|e| format!("could not apply update: {e}"))?;
        }
        Ok(std::mem::take(&mut *self.changes.lock().unwrap()))
    }
}

/// Apply `changes` to a plain string the way an editor would (UTF-16 offsets).
pub fn apply_changes(text: &str, changes: &[TextChange]) -> String {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    for c in changes {
        let start = (c.pos as usize).min(units.len());
        let end = (start + c.remove as usize).min(units.len());
        units.splice(start..end, c.insert.encode_utf16());
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sync(from: &SharedDoc, to: &mut SharedDoc) -> Vec<TextChange> {
        to.apply_update(&from.state()).unwrap()
    }

    #[test]
    fn local_edits_produce_updates_that_rebuild_the_text() {
        let mut a = SharedDoc::new();
        let u1 = a.replace(0, 0, "hello world");
        let u2 = a.replace(5, 0, ",");
        let u3 = a.replace(0, 1, "J");
        assert_eq!(a.text(), "Jello, world");

        let mut b = SharedDoc::new();
        for u in [u1, u2, u3] {
            b.apply_update(&u).unwrap();
        }
        assert_eq!(b.text(), "Jello, world");
    }

    #[test]
    fn concurrent_edits_converge_on_both_sides() {
        let mut a = SharedDoc::new();
        let seed = a.replace(0, 0, "abcdef");
        let mut b = SharedDoc::new();
        b.apply_update(&seed).unwrap();

        // Both edit the same spot without seeing each other.
        let ua = a.replace(3, 0, "XX");
        let ub = b.replace(3, 0, "YY");
        let ua2 = a.replace(0, 1, "");
        a.apply_update(&ub).unwrap();
        b.apply_update(&ua).unwrap();
        b.apply_update(&ua2).unwrap();
        assert_eq!(a.text(), b.text(), "replicas must converge");
        assert!(a.text().contains("XX") && a.text().contains("YY"));
        assert!(!a.text().starts_with('a'));
    }

    #[test]
    fn remote_changes_can_patch_a_plain_copy() {
        let mut a = SharedDoc::new();
        let mut b = SharedDoc::new();
        let mut mirror = String::new();

        for (pos, remove, insert) in [(0, 0, "fn main() {}"), (11, 0, " println!(); "), (3, 4, "start"), (0, 0, "// hi\n")] {
            let update = a.replace(pos, remove, insert);
            let changes = b.apply_update(&update).unwrap();
            mirror = apply_changes(&mirror, &changes);
            assert_eq!(mirror, b.text(), "after {insert:?}");
        }
        assert_eq!(mirror, a.text());
    }

    #[test]
    fn a_late_joiner_gets_everything_from_state() {
        let mut a = SharedDoc::new();
        a.replace(0, 0, "line one\nline two\n");
        a.replace(9, 0, "inserted ");
        let mut late = SharedDoc::new();
        let changes = sync(&a, &mut late);
        assert_eq!(late.text(), a.text());
        assert_eq!(apply_changes("", &changes), a.text());
    }

    #[test]
    fn offsets_are_utf16_so_astral_characters_count_twice() {
        assert_eq!(utf16_len("a😀b"), 4);
        let mut a = SharedDoc::new();
        a.replace(0, 0, "a😀b");
        assert_eq!(a.len(), 4);
        // Insert after the emoji (offset 3).
        let u = a.replace(3, 0, "!");
        assert_eq!(a.text(), "a😀!b");
        let mut b = SharedDoc::new();
        b.apply_update(&a.state()).unwrap();
        assert_eq!(b.text(), "a😀!b");
        let _ = u;
    }

    #[test]
    fn garbage_updates_are_rejected_not_panics() {
        let mut a = SharedDoc::new();
        assert!(a.apply_update(&[255, 255, 255]).is_err());
        assert!(a.is_empty());
    }
}
