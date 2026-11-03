//! Optional shared-secret access control.
//!
//! With no secrets configured the relay is open (as before). A server-wide
//! token applies to every room; a per-room token overrides it for that room.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Auth {
    pub server_token: Option<String>,
    pub room_tokens: HashMap<String, String>,
}

impl Auth {
    pub fn open() -> Auth {
        Auth::default()
    }

    pub fn is_open(&self) -> bool {
        self.server_token.is_none() && self.room_tokens.is_empty()
    }

    fn required(&self, room: &str) -> Option<&str> {
        self.room_tokens.get(room).or(self.server_token.as_ref()).map(String::as_str)
    }

    /// `Ok` when `given` satisfies what `room` requires; otherwise a message for the user.
    pub fn check(&self, room: &str, given: Option<&str>) -> Result<(), String> {
        let Some(need) = self.required(room) else { return Ok(()) };
        match given {
            None | Some("") => Err("this relay needs a password; enter it in the Password field".into()),
            Some(g) if constant_time_eq(g.as_bytes(), need.as_bytes()) => Ok(()),
            Some(_) => Err("wrong password for this relay or room".into()),
        }
    }
}

/// Compare without leaking where the first difference is.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u64;
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as u64;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_servers_accept_anyone() {
        let a = Auth::open();
        assert!(a.check("r", None).is_ok() && a.check("r", Some("whatever")).is_ok());
    }

    #[test]
    fn server_token_is_required_everywhere() {
        let a = Auth { server_token: Some("s3cret".into()), ..Auth::default() };
        assert!(a.check("r", None).unwrap_err().contains("password"));
        assert!(a.check("r", Some("nope")).unwrap_err().contains("wrong"));
        assert!(a.check("r", Some("s3cret")).is_ok());
    }

    #[test]
    fn room_token_overrides_server_token() {
        let mut a = Auth { server_token: Some("s".into()), ..Auth::default() };
        a.room_tokens.insert("private".into(), "p".into());
        assert!(a.check("private", Some("s")).is_err());
        assert!(a.check("private", Some("p")).is_ok());
        assert!(a.check("other", Some("s")).is_ok());
    }

    #[test]
    fn comparison_handles_lengths() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
