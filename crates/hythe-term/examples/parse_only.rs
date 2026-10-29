//! Emulator-only speed: feed bytes straight into the terminal with no PTY or
//! child process, to show how much of the end-to-end cost is the emulator.
//!
//!     cargo run --release --example parse_only -- [megabytes]

use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

fn main() {
    let mb: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let line = "the quick brown fox jumps over the lazy dog 0123456789 \x1b[32mgreen\x1b[0m text\r\n";
    let mut data = Vec::with_capacity(mb * 1024 * 1024 + line.len());
    while data.len() < mb * 1024 * 1024 {
        data.extend_from_slice(line.as_bytes());
    }

    let mut term = Term::new(Config::default(), &TermSize::new(200, 50), VoidListener);
    let mut parser: Processor = Processor::new();

    let start = Instant::now();
    // 64 KiB chunks, like the PTY reader hands over.
    for chunk in data.chunks(65536) {
        parser.advance(&mut term, chunk);
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "parsed {:.0} MB in {:.2}s = {:.0} MB/s (emulator only)",
        data.len() as f64 / 1e6,
        secs,
        data.len() as f64 / 1e6 / secs
    );
}
