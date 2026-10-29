//! Source control: state and message handling. Drawing lives in `scm_view.rs`.

use hythe_git::{Branch, Change, DiffLine, Repo};
use iced::Task;

use crate::{App, Message};

#[derive(Default)]
pub struct Scm {
    /// The Changes view is showing instead of the editor panes.
    pub open: bool,
    pub changes: Vec<Change>,
    /// File whose diff is shown, and whether it's the staged side.
    pub selected: Option<(String, bool)>,
    pub diff: Vec<DiffLine>,
    pub message: String,
    /// A push/pull/fetch is running.
    pub busy: Option<&'static str>,
    pub branches: Vec<Branch>,
    pub branch_menu: bool,
    pub new_branch: String,
    pub ahead_behind: Option<(u32, u32)>,
}

impl Scm {
    pub fn staged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.staged.is_some())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.unstaged.is_some())
    }

    pub fn reload(&mut self, repo: &Repo) {
        self.changes = repo.status().unwrap_or_default();
        self.ahead_behind = repo.ahead_behind();
        // Keep the selection if it still exists on that side; otherwise show nothing.
        let still_there = self.selected.as_ref().is_some_and(|(path, staged)| {
            self.changes.iter().any(|c| &c.path == path && if *staged { c.staged.is_some() } else { c.unstaged.is_some() })
        });
        if !still_there {
            self.selected = None;
            self.diff.clear();
        }
        if let Some((path, staged)) = self.selected.clone() {
            self.diff = repo.diff(&path, staged).unwrap_or_default();
        }
    }

    pub fn select(&mut self, repo: &Repo, path: String, staged: bool) {
        self.diff = repo.diff(&path, staged).unwrap_or_default();
        self.selected = Some((path, staged));
    }
}

#[derive(Debug, Clone)]
pub enum ScmMsg {
    Show,
    Hide,
    Refresh,
    Select { path: String, staged: bool },
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Text(String),
    Commit,
    Push,
    Pull,
    Fetch,
    /// A background git command finished.
    Done(Result<String, String>),
    OpenFile(String),
    BranchMenu(bool),
    Checkout(String),
    NewBranchText(String),
    CreateBranch,
    ToggleBlame,
}

/// Run a slow git command off the UI thread.
fn background(label: &'static str, repo: Repo, op: fn(&Repo) -> hythe_git::Result<String>) -> (&'static str, Task<Message>) {
    let task = Task::perform(
        async move { tokio::task::spawn_blocking(move || op(&repo)).await.unwrap_or_else(|e| Err(e.to_string())) },
        |r| Message::Scm(ScmMsg::Done(r)),
    );
    (label, task)
}

impl App {
    /// Open or refresh whatever source-control state is on screen.
    pub(crate) fn scm_reload(&mut self) {
        if let Some(repo) = self.repo.clone() {
            if self.scm.open || self.scm.branch_menu {
                self.scm.reload(&repo);
            }
        }
    }

    pub(crate) fn scm_update(&mut self, msg: ScmMsg) -> Task<Message> {
        let Some(repo) = self.repo.clone() else {
            if matches!(msg, ScmMsg::Show) {
                self.status = "This folder isn't a git repository".into();
            }
            return Task::none();
        };
        match msg {
            ScmMsg::Show => {
                self.scm.open = true;
                self.focus_term = false;
                self.scm.reload(&repo);
            }
            ScmMsg::Hide => self.scm.open = false,
            ScmMsg::Refresh => {
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::Select { path, staged } => self.scm.select(&repo, path, staged),
            ScmMsg::Stage(path) => self.scm_op(&repo, |r, p| r.stage(&[p]), &path),
            ScmMsg::Unstage(path) => self.scm_op(&repo, |r, p| r.unstage(&[p]), &path),
            ScmMsg::StageAll => {
                let paths: Vec<String> = self.scm.unstaged().map(|c| c.path.clone()).collect();
                let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
                if let Err(e) = repo.stage(&refs) {
                    self.status = e;
                }
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::UnstageAll => {
                let paths: Vec<String> = self.scm.staged().map(|c| c.path.clone()).collect();
                let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
                if let Err(e) = repo.unstage(&refs) {
                    self.status = e;
                }
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::Text(t) => self.scm.message = t,
            ScmMsg::Commit => {
                if self.scm.staged().next().is_none() {
                    self.status = "Stage some changes to commit".into();
                } else {
                    match repo.commit(&self.scm.message) {
                        Ok(sha) => {
                            self.status = format!("Committed {sha}");
                            self.scm.message.clear();
                        }
                        Err(e) => self.status = e,
                    }
                    self.refresh_git();
                    self.scm.reload(&repo);
                }
            }
            ScmMsg::Push => return self.scm_background(repo, "Pushing…", |r| r.push()),
            ScmMsg::Pull => return self.scm_background(repo, "Pulling…", |r| r.pull()),
            ScmMsg::Fetch => return self.scm_background(repo, "Fetching…", |r| r.fetch()),
            ScmMsg::Done(result) => {
                self.scm.busy = None;
                self.status = match result {
                    Ok(out) => out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Done").trim().to_string(),
                    Err(e) => e.lines().next().unwrap_or("git failed").to_string(),
                };
                self.reload_open_files();
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::OpenFile(path) => {
                let full = repo.root().join(&path);
                self.scm.open = false;
                self.open_file(full);
            }
            ScmMsg::BranchMenu(open) => {
                self.scm.branch_menu = open;
                if open {
                    self.scm.branches = repo.branches().unwrap_or_default();
                    self.scm.new_branch.clear();
                }
            }
            ScmMsg::Checkout(name) => {
                self.scm.branch_menu = false;
                match repo.checkout(&name) {
                    Ok(()) => {
                        self.status = format!("Switched to {name}");
                        self.reload_open_files();
                    }
                    Err(e) => self.status = e.lines().next().unwrap_or("checkout failed").to_string(),
                }
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::NewBranchText(t) => self.scm.new_branch = t,
            ScmMsg::CreateBranch => {
                let name = self.scm.new_branch.trim().to_string();
                match repo.create_branch(&name) {
                    Ok(()) => {
                        self.status = format!("Created and switched to {name}");
                        self.scm.branch_menu = false;
                        self.scm.new_branch.clear();
                    }
                    Err(e) => self.status = e.lines().next().unwrap_or("could not create branch").to_string(),
                }
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::ToggleBlame => {
                self.blame_on = !self.blame_on;
                self.status = if self.blame_on { "Inline blame on".into() } else { "Inline blame off".into() };
                for i in 0..self.docs.len() {
                    self.load_blame(i);
                }
            }
        }
        Task::none()
    }

    fn scm_op(&mut self, repo: &Repo, op: fn(&Repo, &str) -> hythe_git::Result<()>, path: &str) {
        if let Err(e) = op(repo, path) {
            self.status = e;
        }
        self.refresh_git();
        self.scm.reload(repo);
    }

    fn scm_background(&mut self, repo: Repo, label: &'static str, op: fn(&Repo) -> hythe_git::Result<String>) -> Task<Message> {
        if self.scm.busy.is_some() {
            return Task::none();
        }
        let (label, task) = background(label, repo, op);
        self.scm.busy = Some(label);
        self.status = label.into();
        task
    }

    /// Re-read open files that changed on disk (after a pull or checkout).
    fn reload_open_files(&mut self) {
        for d in self.docs.iter_mut().filter(|d| !d.dirty) {
            if let Some(path) = d.path.clone() {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if text != d.buffer.text() {
                        d.replace_text(&text, &mut self.highlighter);
                    }
                }
            }
        }
        for i in 0..self.docs.len() {
            self.load_blame(i);
        }
    }

    /// Refresh inline blame for one document (when blame is on).
    pub(crate) fn load_blame(&mut self, index: usize) {
        let Some(doc) = self.docs.get_mut(index) else { return };
        doc.blame = None;
        if !self.blame_on {
            return;
        }
        let (Some(repo), Some(path)) = (&self.repo, doc.path.clone()) else { return };
        let Ok(rel) = path.strip_prefix(repo.root()) else { return };
        let rel = rel.to_string_lossy().replace('\\', "/");
        doc.blame = repo.blame(&rel).ok().filter(|b| !b.is_empty());
    }
}
