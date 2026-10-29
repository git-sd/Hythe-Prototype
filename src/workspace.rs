//! One workspace per repository. The app's live fields hold the open one;
//! the others are parked here with their tabs, terminals and git state intact,
//! so a repo's terminals and agents never leak into another repo.

use std::path::PathBuf;

use hythe_term::{SessionId, Snapshot, TerminalManager};

use crate::collab::Collab;
use crate::doc::Doc;
use crate::explorer::Node;
use crate::panes::Panes;
use crate::scm::Scm;
use crate::state::GitInfo;
use crate::{ask_unsaved, App, Unsaved};

/// Everything that belongs to one repository.
pub struct Workspace {
    pub docs: Vec<Doc>,
    pub panes: Panes,
    pub root: Option<Node>,
    pub terms: TerminalManager,
    pub term_active: Option<SessionId>,
    pub term_snap: Option<Snapshot>,
    pub repo: Option<hythe_git::Repo>,
    pub git: Option<GitInfo>,
    pub scm: Scm,
    pub focus_term: bool,
    pub collab: Option<Collab>,
}

impl Workspace {
    pub fn empty() -> Workspace {
        Workspace {
            docs: Vec::new(),
            panes: Panes::new(),
            root: None,
            terms: TerminalManager::default(),
            term_active: None,
            term_snap: None,
            repo: None,
            git: None,
            scm: Scm::default(),
            focus_term: false,
            collab: None,
        }
    }

    pub fn for_folder(path: PathBuf) -> Workspace {
        Workspace { root: Some(Node::root(path)), ..Workspace::empty() }
    }

    pub fn has_dirty_docs(&self) -> bool {
        self.docs.iter().any(|d| d.dirty)
    }

    /// A terminal or agent is still running in it.
    pub fn has_running_terminal(&self) -> bool {
        self.terms.sessions().iter().any(|s| s.is_running())
    }
}

/// A repository in the rail. `ws` holds its state while it isn't the open one.
pub struct Project {
    pub path: PathBuf,
    pub ws: Option<Workspace>,
}

impl App {
    fn take_workspace(&mut self) -> Workspace {
        Workspace {
            docs: std::mem::take(&mut self.docs),
            panes: std::mem::replace(&mut self.panes, Panes::new()),
            root: self.root.take(),
            terms: std::mem::take(&mut self.terms),
            term_active: self.term_active.take(),
            term_snap: self.term_snap.take(),
            repo: self.repo.take(),
            git: self.git.take(),
            scm: std::mem::take(&mut self.scm),
            focus_term: std::mem::take(&mut self.focus_term),
            collab: self.collab.take(),
        }
    }

    fn put_workspace(&mut self, ws: Workspace) {
        self.docs = ws.docs;
        self.panes = ws.panes;
        self.root = ws.root;
        self.terms = ws.terms;
        self.term_active = ws.term_active;
        self.term_snap = ws.term_snap;
        self.repo = ws.repo;
        self.git = ws.git;
        self.scm = ws.scm;
        self.focus_term = ws.focus_term;
        self.collab = ws.collab;
    }

    /// Open the repository at rail position `target`, or the home screen for `None`.
    pub(crate) fn switch_project(&mut self, target: Option<usize>) {
        if target == self.active_project || target.is_some_and(|i| i >= self.projects.len()) {
            return;
        }
        self.completion = None;
        self.menu = None;
        self.palette = None;
        // Chat belongs to one repository.
        self.chat = crate::chat::Chat::default();

        let parked = self.take_workspace();
        match self.active_project {
            Some(i) => self.projects[i].ws = Some(parked),
            None => self.home = Some(parked),
        }
        let next = match target {
            Some(i) => self.projects[i]
                .ws
                .take()
                .unwrap_or_else(|| Workspace::for_folder(self.projects[i].path.clone())),
            None => self.home.take().unwrap_or_else(Workspace::empty),
        };
        self.put_workspace(next);
        self.active_project = target;

        self.persisted.active = target.map(|i| self.projects[i].path.clone());
        self.persisted.save();
        self.refresh_git();
        if self.term_active.is_some() {
            self.refresh_term_snapshot();
        }
    }

    /// Add a repository to the rail (or switch to it if it's already there) and open it.
    pub(crate) fn open_folder(&mut self, path: PathBuf) {
        let index = self.persisted.add_project(&path);
        if self.projects.len() <= index || self.projects[index].path != path {
            self.projects.insert(index.min(self.projects.len()), Project { path, ws: None });
        }
        self.persisted.save();
        self.show_sidebar = true;
        self.switch_project(Some(index));
    }

    /// Take a repository out of the rail, ending its terminals. Asks first if it has unsaved edits.
    pub(crate) fn close_project(&mut self, index: usize) {
        if index >= self.projects.len() {
            return;
        }
        let name = self.project_name(index);
        let dirty: Vec<String> = if self.active_project == Some(index) {
            self.docs.iter().filter(|d| d.dirty).map(|d| d.name()).collect()
        } else {
            self.projects[index]
                .ws
                .as_ref()
                .map(|w| w.docs.iter().filter(|d| d.dirty).map(|d| d.name()).collect())
                .unwrap_or_default()
        };
        if !dirty.is_empty() {
            let body = format!("{name} has unsaved changes:\n{}\n\nYes saves them, No discards them.", dirty.join("\n"));
            match ask_unsaved(&body, "Save All") {
                Unsaved::Cancel => return,
                Unsaved::Discard => {}
                Unsaved::Save => {
                    let docs: Vec<&mut Doc> = if self.active_project == Some(index) {
                        self.docs.iter_mut().collect()
                    } else {
                        self.projects[index].ws.iter_mut().flat_map(|w| w.docs.iter_mut()).collect()
                    };
                    for d in docs.into_iter().filter(|d| d.dirty) {
                        if let Err(e) = d.save() {
                            self.status = e;
                            return;
                        }
                    }
                }
            }
        }
        if self.active_project == Some(index) {
            // Park it first so the dialog-free path below can drop it.
            self.switch_project(None);
        }
        let removed = self.projects.remove(index);
        if let Some(a) = self.active_project {
            if a > index {
                self.active_project = Some(a - 1);
            }
        }
        self.persisted.remove_project(&removed.path);
        self.persisted.save();
        self.status = format!("Closed {}", removed.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        // Dropping `removed` shuts down its terminals.
    }

    pub(crate) fn project_name(&self, index: usize) -> String {
        self.projects
            .get(index)
            .and_then(|p| p.path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Unsaved files in workspaces that aren't open right now, as "repo / file".
    pub(crate) fn parked_dirty_docs(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, p) in self.projects.iter().enumerate() {
            if let Some(ws) = &p.ws {
                out.extend(ws.docs.iter().filter(|d| d.dirty).map(|d| format!("{} / {}", self.project_name(i), d.name())));
            }
        }
        if let Some(home) = &self.home {
            out.extend(home.docs.iter().filter(|d| d.dirty).map(|d| d.name()));
        }
        out
    }

    /// Save every unsaved file in parked workspaces.
    pub(crate) fn save_parked_docs(&mut self) -> Result<(), String> {
        let mut docs: Vec<&mut Doc> = Vec::new();
        for p in self.projects.iter_mut() {
            if let Some(ws) = p.ws.as_mut() {
                docs.extend(ws.docs.iter_mut());
            }
        }
        if let Some(home) = self.home.as_mut() {
            docs.extend(home.docs.iter_mut());
        }
        for d in docs.into_iter().filter(|d| d.dirty) {
            d.save()?;
        }
        Ok(())
    }
}
