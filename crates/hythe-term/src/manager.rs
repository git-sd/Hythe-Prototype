use std::io;
use std::sync::Arc;

use crate::palette::Palette;
use crate::profile::{Profile, SpawnConfig};
use crate::session::{Session, SessionId};

/// Summary of a session for listing in a UI.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub title: String,
    pub running: bool,
    pub exit_code: Option<i32>,
}

/// Owns a set of terminal sessions: shells and CLI agents side by side.
pub struct TerminalManager {
    sessions: Vec<Arc<Session>>,
    next_id: SessionId,
    palette: Arc<Palette>,
    cols: u16,
    rows: u16,
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new(Palette::default())
    }
}

impl TerminalManager {
    pub fn new(palette: Palette) -> Self {
        Self { sessions: Vec::new(), next_id: 1, palette: Arc::new(palette), cols: 100, rows: 30 }
    }

    /// Size new sessions start with, until the UI resizes them.
    pub fn set_default_size(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(2);
        self.rows = rows.max(1);
    }

    pub fn spawn(&mut self, cfg: &SpawnConfig) -> io::Result<SessionId> {
        let id = self.next_id;
        let session = Session::spawn(id, cfg, self.cols, self.rows, self.palette.clone())?;
        self.next_id += 1;
        self.sessions.push(Arc::new(session));
        Ok(id)
    }

    /// Launch a profile (a shell or an agent) in `cwd`.
    pub fn spawn_profile(
        &mut self,
        profile: &Profile,
        cwd: Option<std::path::PathBuf>,
    ) -> io::Result<SessionId> {
        let mut cfg = profile.spawn_config();
        cfg.cwd = cwd;
        self.spawn(&cfg)
    }

    pub fn get(&self, id: SessionId) -> Option<Arc<Session>> {
        self.sessions.iter().find(|s| s.id() == id).cloned()
    }

    /// Sessions in creation order.
    pub fn sessions(&self) -> &[Arc<Session>] {
        &self.sessions
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .iter()
            .map(|s| {
                let exit = s.exit_code();
                SessionInfo {
                    id: s.id(),
                    name: s.name().to_string(),
                    title: s.title(),
                    running: exit.is_none(),
                    exit_code: exit.flatten(),
                }
            })
            .collect()
    }

    /// Move session `id` to where session `target` is, shifting the ones between.
    pub fn move_session(&mut self, id: SessionId, target: SessionId) {
        let pos = |id: SessionId| self.sessions.iter().position(|s| s.id() == id);
        let (Some(from), Some(to)) = (pos(id), pos(target)) else { return };
        let session = self.sessions.remove(from);
        self.sessions.insert(to, session);
    }

    /// Remove a session, terminating its process. Returns false if `id` is unknown.
    pub fn close(&mut self, id: SessionId) -> bool {
        match self.sessions.iter().position(|s| s.id() == id) {
            Some(i) => {
                self.sessions.remove(i).shutdown();
                true
            }
            None => false,
        }
    }

    /// Ids of sessions whose output changed since the last call.
    pub fn take_dirty(&self) -> Vec<SessionId> {
        self.sessions.iter().filter(|s| s.take_dirty()).map(|s| s.id()).collect()
    }
}
