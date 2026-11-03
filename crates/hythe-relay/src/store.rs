//! Optional on-disk persistence: one file per document, written atomically.
//!
//! Layout: `<dir>/<room hash>/<doc hash>.doc`. Each file is a magic line, one
//! JSON line naming the room and document, then the raw CRDT state. Names are
//! hashed for the file names (so no user-controlled path is ever used), and the
//! header is the source of truth when loading.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"HYTHE-RELAY-DOC 1\n";

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

fn fnv(bytes: &[u8], basis: u64) -> u64 {
    bytes.iter().fold(basis, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// 128 bits of FNV-1a (two bases), as hex. Stable across Rust versions.
fn hash_name(s: &str) -> String {
    format!("{:016x}{:016x}", fnv(s.as_bytes(), 0xcbf29ce484222325), fnv(s.as_bytes(), 0x84222325cbf29ce4))
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Store> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Store { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn room_dir(&self, room: &str) -> PathBuf {
        self.dir.join(hash_name(room))
    }

    /// Write `state` for (`room`, `doc`) so that a crash leaves either the old or new file.
    pub fn save(&self, room: &str, doc: &str, state: &[u8]) -> io::Result<()> {
        let dir = self.room_dir(room);
        fs::create_dir_all(&dir)?;
        let name = hash_name(doc);
        let tmp = dir.join(format!("{name}.tmp"));
        let header = serde_json::json!({ "room": room, "doc": doc }).to_string();
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(MAGIC)?;
            f.write_all(header.as_bytes())?;
            f.write_all(b"\n")?;
            f.write_all(state)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, dir.join(format!("{name}.doc")))?;
        // Best effort: make the rename itself durable.
        if let Ok(d) = fs::File::open(&dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    /// Every document saved for `room`. Unreadable or foreign files are skipped.
    pub fn load_room(&self, room: &str) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(self.room_dir(room)) else { return out };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("doc") {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            if let Some((r, doc, state)) = parse(&bytes) {
                if r == room {
                    out.push((doc, state));
                }
            }
        }
        out
    }
}

fn parse(bytes: &[u8]) -> Option<(String, String, Vec<u8>)> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let nl = rest.iter().position(|b| *b == b'\n')?;
    let header: serde_json::Value = serde_json::from_slice(&rest[..nl]).ok()?;
    Some((header["room"].as_str()?.to_string(), header["doc"].as_str()?.to_string(), rest[nl + 1..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hythe-store-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn saves_and_loads_per_room() {
        let dir = tmpdir("roundtrip");
        let s = Store::open(&dir).unwrap();
        s.save("room/../evil", "src/a.rs", b"\x00\x01\n\x02").unwrap();
        s.save("room/../evil", "b.rs", b"second").unwrap();
        s.save("other", "c.rs", b"third").unwrap();
        s.save("room/../evil", "b.rs", b"second v2").unwrap(); // overwrite
        let mut docs = s.load_room("room/../evil");
        docs.sort();
        assert_eq!(docs, vec![("b.rs".to_string(), b"second v2".to_vec()), ("src/a.rs".to_string(), b"\x00\x01\n\x02".to_vec())]);
        assert_eq!(s.load_room("other").len(), 1);
        assert!(s.load_room("missing").is_empty());
        let leftovers = fs::read_dir(dir.join(hash_name("other"))).unwrap().flatten().filter(|e| e.path().extension().unwrap() == "tmp").count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_files_are_skipped() {
        let dir = tmpdir("corrupt");
        let s = Store::open(&dir).unwrap();
        s.save("r", "good", b"ok").unwrap();
        fs::write(dir.join(hash_name("r")).join("junk.doc"), b"not a doc").unwrap();
        assert_eq!(s.load_room("r").len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
