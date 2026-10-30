//! Source control: state and message handling. Drawing lives in `scm_view.rs`.

use hythe_git::{Branch, Change, Commit, CommitFile, DiffLine, LineKind, Repo};
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
    /// The diff of the chosen file or commit is showing beside the code.
    pub show_diff: bool,
    /// Every changed file with its diff against HEAD, for the Changes view.
    pub all_diffs: Vec<(String, Vec<DiffLine>)>,
    /// Syntax colours for `all_diffs`, file by file and line by line: runs of (text,
    /// colour) for lines that exist in the current file, `None` for the rest.
    pub diff_spans: Vec<Vec<Option<Vec<(String, iced::Color)>>>>,
    /// Whether the right side shows a changed file or a commit.
    pub tab: ScmTab,
    pub commits: Vec<Commit>,
    /// Commit whose details are shown (by full hash).
    pub selected_commit: Option<String>,
    pub commit_files: Vec<CommitFile>,
    pub commit_file: Option<String>,
    pub commit_diff: Vec<DiffLine>,
}

/// What Source Control is drawn into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScmPlace {
    Window,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScmTab {
    #[default]
    Changes,
    History,
}

/// How many commits the History tab loads.
const HISTORY_LIMIT: usize = 200;

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
        self.load_history(repo);
    }

    /// Read the commit list, keeping the selected commit if it's still there.
    pub fn load_history(&mut self, repo: &Repo) {
        self.commits = repo.log(HISTORY_LIMIT).unwrap_or_default();
        // Keep showing the same commit; only pick one when none is showing (or it vanished).
        let still_there = self.selected_commit.as_ref().is_some_and(|sha| self.commits.iter().any(|c| &c.sha == sha));
        if still_there {
            return;
        }
        match self.commits.first().map(|c| c.sha.clone()) {
            Some(sha) => self.select_commit(repo, sha),
            None => {
                self.selected_commit = None;
                self.commit_files.clear();
                self.commit_file = None;
                self.commit_diff.clear();
            }
        }
    }

    pub fn select_commit(&mut self, repo: &Repo, sha: String) {
        self.commit_files = repo.commit_files(&sha).unwrap_or_default();
        let first = self.commit_files.first().map(|f| f.path.clone());
        self.selected_commit = Some(sha);
        match first {
            Some(path) => self.select_commit_file(repo, path),
            None => {
                self.commit_file = None;
                self.commit_diff.clear();
            }
        }
    }

    pub fn select_commit_file(&mut self, repo: &Repo, path: String) {
        if let Some(sha) = &self.selected_commit {
            self.commit_diff = repo.commit_diff(sha, &path).unwrap_or_default();
        }
        self.commit_file = Some(path);
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
    /// Close the diff, back to the code.
    CloseDiff,
    /// Diffs of every changed file arrived (for the Changes view).
    AllDiffs(Vec<(String, Vec<DiffLine>)>),
    /// Open Source Control in its own window.
    PopOut,
    /// Bring it back from its own window.
    PopIn,
    Refresh,
    Select { path: String, staged: bool },
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Text(String),
    Commit,
    /// Commit, then push.
    CommitPush,
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
    SelectCommit(String),
    SelectCommitFile(String),
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
            if self.scm.open || self.scm_window.is_some() || self.scm.branch_menu || self.changes_visible() {
                self.scm.reload(&repo);
            }
        }
    }

    /// Commit with the message in the box. With nothing staged, everything
    /// changed is committed, like VS Code's smart commit. True if it worked.
    pub(crate) fn scm_commit(&mut self, repo: &Repo) -> bool {
        if self.scm.message.trim().is_empty() {
            self.status = "Write a commit message first".into();
            return false;
        }
        if self.scm.staged().next().is_none() {
            if self.scm.changes.is_empty() {
                // A clean tree still takes a message, as a commit with no file changes.
                let ok = match repo.commit_empty(&self.scm.message) {
                    Ok(sha) => {
                        self.status = format!("Committed message {sha}");
                        self.scm.message.clear();
                        true
                    }
                    Err(e) => {
                        self.status = e;
                        false
                    }
                };
                self.refresh_git();
                self.scm.reload(repo);
                return ok;
            }
            let paths: Vec<String> = self.scm.changes.iter().map(|c| c.path.clone()).collect();
            let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            if let Err(e) = repo.stage(&refs) {
                self.status = e;
                return false;
            }
        }
        let ok = match repo.commit(&self.scm.message) {
            Ok(sha) => {
                self.status = format!("Committed {sha}");
                self.scm.message.clear();
                true
            }
            Err(e) => {
                self.status = e;
                false
            }
        };
        self.refresh_git();
        self.scm.reload(repo);
        ok
    }

    /// Colour one file's diff like the editor would: the current file is highlighted once,
    /// and each added or unchanged line takes its colours from there.
    fn colour_diff(&mut self, root: &std::path::Path, path: &str, lines: &[DiffLine]) -> Vec<Option<Vec<(String, iced::Color)>>> {
        let full = root.join(path);
        let syntax = crate::highlight::Syntax::for_file(hythe_complete::Language::from_path(&full), Some(&full));
        let source = std::fs::read_to_string(&full).unwrap_or_default();
        let classes = match syntax {
            Some(s) if source.len() <= crate::highlight::MAX_HIGHLIGHT_BYTES => self.highlighter.highlight(s, &source),
            _ => Vec::new(),
        };
        // Byte range of each line of the file.
        let mut starts = vec![0usize];
        starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        lines
            .iter()
            .map(|l| {
                let n = l.new? as usize;
                if classes.is_empty() || l.kind == LineKind::Removed || n == 0 || n > starts.len() {
                    return None;
                }
                let start = starts[n - 1];
                let end = starts.get(n).map(|e| e - 1).unwrap_or(source.len());
                let line = source.get(start..end)?.trim_end_matches('\r');
                let mut runs: Vec<(String, iced::Color)> = Vec::new();
                for (offset, ch) in line.char_indices() {
                    let color = crate::editor::class_color(classes.get(start + offset).copied().unwrap_or(crate::highlight::NONE));
                    match runs.last_mut() {
                        Some((text, c)) if *c == color => text.push(ch),
                        _ => runs.push((ch.to_string(), color)),
                    }
                }
                Some(runs)
            })
            .collect()
    }

    /// Read every changed file's diff off the UI thread, for the Changes view.
    pub(crate) fn load_all_diffs(&self) -> Task<Message> {
        let Some(repo) = self.repo.clone() else { return Task::none() };
        let paths: Vec<String> = self.scm.changes.iter().map(|c| c.path.clone()).collect();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    paths.into_iter().map(|p| {
                        let lines = repo.diff_head(&p).unwrap_or_default();
                        (p, lines)
                    }).collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default()
            },
            |d| Message::Scm(ScmMsg::AllDiffs(d)),
        )
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
                self.scm.show_diff = false;
                self.focus_term = false;
                self.scm.reload(&repo);
                // Open on what needs attention: working changes if there are any, else history.
                self.scm.tab = if self.scm.changes.is_empty() { ScmTab::History } else { ScmTab::Changes };
            }
            ScmMsg::CloseDiff => self.scm.show_diff = false,
            ScmMsg::AllDiffs(diffs) => {
                // The diffs are re-read every few seconds; only colour them again when they changed.
                if diffs != self.scm.all_diffs {
                    self.scm.diff_spans = diffs.iter().map(|(path, lines)| self.colour_diff(repo.root(), path, lines)).collect();
                    self.scm.all_diffs = diffs;
                }
            }
            ScmMsg::Hide => {
                self.scm.open = false;
                self.scm.show_diff = false;
                if let Some(id) = self.scm_window.take() {
                    return iced::window::close(id);
                }
            }
            ScmMsg::PopOut => {
                if self.scm_window.is_some() {
                    return Task::none();
                }
                let (id, open) = iced::window::open(iced::window::Settings {
                    size: iced::Size::new(1000.0, 680.0),
                    min_size: Some(iced::Size::new(640.0, 360.0)),
                    position: iced::window::Position::Centered,
                    exit_on_close_request: false,
                    ..iced::window::Settings::default()
                });
                self.scm_window = Some(id);
                self.scm.open = true;
                return open.map(|_| Message::Noop);
            }
            ScmMsg::PopIn => {
                if let Some(id) = self.scm_window.take() {
                    return iced::window::close(id);
                }
            }
            ScmMsg::SelectCommit(sha) => {
                self.scm.show_diff = true;
                self.scm.tab = ScmTab::History;
                self.scm.select_commit(&repo, sha);
            }
            ScmMsg::SelectCommitFile(path) => {
                self.scm.show_diff = true;
                self.scm.select_commit_file(&repo, path);
            }
            ScmMsg::Refresh => {
                self.refresh_git();
                self.scm.reload(&repo);
            }
            ScmMsg::Select { path, staged } => {
                self.scm.show_diff = true;
                self.scm.tab = ScmTab::Changes;
                self.scm.select(&repo, path, staged);
            }
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
                self.scm_commit(&repo);
            }
            ScmMsg::CommitPush => {
                if self.scm_commit(&repo) {
                    return self.scm_background(repo, "Pushing…", |r| r.push());
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
                if self.scm_window.is_none() {
                    self.scm.open = false;
                }
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
