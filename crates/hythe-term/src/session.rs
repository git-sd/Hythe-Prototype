use std::borrow::Cow;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Rgb as VteRgb;

use crate::input::{self, Key, Mods};
use crate::palette::Palette;
use crate::profile::{find_in_path, SpawnConfig};
use crate::snapshot::{self, Snapshot};

pub type SessionId = u64;

/// Lines of scrollback kept per session.
const SCROLLBACK: usize = 10_000;

/// State the PTY thread and the UI both touch, kept lock-free where it matters.
struct Shared {
    dirty: AtomicBool,
    generation: AtomicU64,
    bell: AtomicBool,
    title: Mutex<String>,
    /// `Some(code)` once the child has exited (`code` is `None` if killed by a signal).
    exit: Mutex<Option<Option<i32>>>,
    size: Mutex<WindowSize>,
    /// Filled in once the event loop exists; used to answer terminal queries.
    sender: OnceLock<EventLoopSender>,
}

#[derive(Clone)]
struct Listener {
    shared: Arc<Shared>,
    palette: Arc<Palette>,
}

impl Listener {
    fn reply(&self, text: String) {
        if let Some(tx) = self.shared.sender.get() {
            let _ = tx.send(Msg::Input(Cow::Owned(text.into_bytes())));
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let s = &self.shared;
        match event {
            Event::Wakeup => {
                s.generation.fetch_add(1, Ordering::Relaxed);
                s.dirty.store(true, Ordering::Release);
            }
            Event::Title(t) => {
                *s.title.lock().unwrap() = t;
                s.dirty.store(true, Ordering::Release);
            }
            Event::ResetTitle => {
                s.title.lock().unwrap().clear();
                s.dirty.store(true, Ordering::Release);
            }
            Event::Bell => s.bell.store(true, Ordering::Release),
            Event::PtyWrite(text) => self.reply(text),
            Event::ColorRequest(index, format) => {
                let p = &self.palette;
                let c = match index {
                    256 => p.fg,
                    257 => p.bg,
                    258 => p.cursor,
                    0..=255 => p.indexed(index as u8),
                    _ => p.fg,
                };
                self.reply(format(VteRgb { r: c.0, g: c.1, b: c.2 }));
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *s.size.lock().unwrap();
                self.reply(format(size));
            }
            Event::ChildExit(status) => {
                *s.exit.lock().unwrap() = Some(status.code());
                s.generation.fetch_add(1, Ordering::Relaxed);
                s.dirty.store(true, Ordering::Release);
            }
            _ => {}
        }
    }
}

/// Grid dimensions handed to the emulator.
struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// One running terminal: a child process on a pseudo-terminal plus the
/// emulated screen it draws. All methods take `&self` and are thread-safe.
pub struct Session {
    id: SessionId,
    name: String,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    shared: Arc<Shared>,
    palette: Arc<Palette>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

fn window_size(cols: u16, rows: u16, cell_w: u16, cell_h: u16) -> WindowSize {
    WindowSize { num_lines: rows, num_cols: cols, cell_width: cell_w, cell_height: cell_h }
}

/// Resolve the program and quote it for the platform's launcher.
fn pty_options(cfg: &SpawnConfig) -> tty::Options {
    let mut opts = tty::Options {
        working_directory: cfg.cwd.clone(),
        drain_on_exit: true,
        ..Default::default()
    };
    opts.env.insert("TERM".into(), "xterm-256color".into());
    opts.env.insert("COLORTERM".into(), "truecolor".into());
    for (k, v) in &cfg.env {
        opts.env.insert(k.clone(), v.clone());
    }

    let resolved = find_in_path(&cfg.program);

    #[cfg(windows)]
    {
        let quote = |arg: &str| -> String {
            if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
                return arg.to_string();
            }
            format!("\"{}\"", arg.replace('"', "\\\""))
        };
        let path = resolved
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| cfg.program.clone());
        let is_script = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
        if is_script {
            // npm-installed CLIs are .cmd shims; run them through cmd.exe.
            let mut line = quote(&path);
            for a in &cfg.args {
                line.push(' ');
                line.push_str(&quote(a));
            }
            opts.shell = Some(tty::Shell::new(
                "cmd.exe".into(),
                vec!["/d".into(), "/s".into(), "/c".into(), format!("\"{line}\"")],
            ));
            opts.escape_args = false;
        } else {
            opts.shell = Some(tty::Shell::new(quote(&path), cfg.args.clone()));
            opts.escape_args = true;
        }
    }
    #[cfg(not(windows))]
    {
        let program = resolved
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| cfg.program.clone());
        opts.shell = Some(tty::Shell::new(program, cfg.args.clone()));
    }
    opts
}

impl Session {
    pub(crate) fn spawn(
        id: SessionId,
        cfg: &SpawnConfig,
        cols: u16,
        rows: u16,
        palette: Arc<Palette>,
    ) -> io::Result<Session> {
        let (cols, rows) = (cols.max(2), rows.max(1));
        let shared = Arc::new(Shared {
            dirty: AtomicBool::new(true),
            generation: AtomicU64::new(1),
            bell: AtomicBool::new(false),
            title: Mutex::new(String::new()),
            exit: Mutex::new(None),
            size: Mutex::new(window_size(cols, rows, 1, 1)),
            sender: OnceLock::new(),
        });
        let listener = Listener { shared: shared.clone(), palette: palette.clone() };

        let config = term::Config { scrolling_history: SCROLLBACK, ..Default::default() };
        let grid = GridSize { cols: cols as usize, rows: rows as usize };
        let term = Arc::new(FairMutex::new(Term::new(config, &grid, listener.clone())));

        let pty = tty::new(&pty_options(cfg), window_size(cols, rows, 1, 1), id)?;
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        let _ = shared.sender.set(sender.clone());
        let handle = event_loop.spawn();

        Ok(Session {
            id,
            name: cfg.display_name(),
            term,
            sender,
            shared,
            palette,
            thread: Mutex::new(Some(std::thread::spawn(move || {
                let _ = handle.join();
            }))),
        })
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Title set by the running program (empty if it never set one).
    pub fn title(&self) -> String {
        self.shared.title.lock().unwrap().clone()
    }

    /// `Some(code)` once the child process has exited.
    pub fn exit_code(&self) -> Option<Option<i32>> {
        *self.shared.exit.lock().unwrap()
    }

    pub fn is_running(&self) -> bool {
        self.exit_code().is_none()
    }

    // --- output ---

    /// True once per batch of new output. Cheap; poll it from a UI timer.
    pub fn take_dirty(&self) -> bool {
        self.shared.dirty.swap(false, Ordering::AcqRel)
    }

    /// Increases whenever the screen changes.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }

    /// True once if the program rang the bell since the last call.
    pub fn take_bell(&self) -> bool {
        self.shared.bell.swap(false, Ordering::AcqRel)
    }

    pub fn snapshot(&self) -> Snapshot {
        let generation = self.generation();
        let title = self.title();
        let term = self.term.lock();
        snapshot::build(&term, &self.palette, generation, title)
    }

    // --- input ---

    /// Send raw bytes to the child's stdin.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    /// Send a key press; returns false if the key has no terminal encoding.
    pub fn send_key(&self, key: &Key, mods: Mods) -> bool {
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        match input::encode_key(key, mods, app_cursor) {
            Some(bytes) => {
                self.scroll_to_bottom();
                self.write(bytes);
                true
            }
            None => false,
        }
    }

    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.scroll_to_bottom();
        self.write(input::encode_paste(text, bracketed));
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        {
            let mut term = self.term.lock();
            if term.columns() == cols as usize && term.screen_lines() == rows as usize {
                return;
            }
            term.resize(GridSize { cols: cols as usize, rows: rows as usize });
        }
        let size = window_size(cols, rows, cell_w, cell_h);
        *self.shared.size.lock().unwrap() = size;
        let _ = self.sender.send(Msg::Resize(size));
        self.shared.dirty.store(true, Ordering::Release);
    }

    // --- scrollback and selection ---

    /// Scroll the view; positive moves up into history.
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
        self.shared.dirty.store(true, Ordering::Release);
    }

    pub fn scroll_to_bottom(&self) {
        let mut term = self.term.lock();
        if term.grid().display_offset() != 0 {
            term.scroll_display(Scroll::Bottom);
            self.shared.dirty.store(true, Ordering::Release);
        }
    }

    fn point(term: &Term<Listener>, row: usize, col: usize) -> Point {
        let line = row as i32 - term.grid().display_offset() as i32;
        Point::new(Line(line), Column(col.min(term.columns().saturating_sub(1))))
    }

    pub fn start_selection(&self, row: usize, col: usize) {
        let mut term = self.term.lock();
        let p = Self::point(&term, row, col);
        term.selection = Some(Selection::new(SelectionType::Simple, p, Side::Left));
        self.shared.dirty.store(true, Ordering::Release);
    }

    pub fn update_selection(&self, row: usize, col: usize) {
        let mut term = self.term.lock();
        let p = Self::point(&term, row, col);
        if let Some(sel) = term.selection.as_mut() {
            sel.update(p, Side::Right);
            self.shared.dirty.store(true, Ordering::Release);
        }
    }

    pub fn clear_selection(&self) {
        let mut term = self.term.lock();
        if term.selection.take().is_some() {
            self.shared.dirty.store(true, Ordering::Release);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    // --- lifecycle ---

    /// Ask the PTY thread to stop; the child is terminated when the pseudo-terminal closes.
    pub fn shutdown(&self) {
        let _ = self.sender.send(Msg::Shutdown);
    }

    /// Poll the screen until `needle` appears or `timeout` passes. Handy for
    /// scripting and tests.
    pub fn wait_for_text(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.snapshot().text().contains(needle) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait for the child to exit; returns its exit code if it finished in time.
    pub fn wait_for_exit(&self, timeout: Duration) -> Option<Option<i32>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(code) = self.exit_code() {
                return Some(code);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(t) = self.thread.lock().unwrap().take() {
            // The PTY thread exits promptly after Shutdown; don't hang teardown if it doesn't.
            let _ = std::thread::spawn(move || {
                let _ = t.join();
            });
        }
    }
}
