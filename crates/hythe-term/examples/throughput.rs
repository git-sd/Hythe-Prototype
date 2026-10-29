//! Rough throughput check: stream a large file through a PTY and time how
//! long the emulator takes to consume it.
//!
//!     cargo run --release --example throughput -- [megabytes]

use std::io::Write;
use std::time::{Duration, Instant};

use hythe_term::{SpawnConfig, TerminalManager};

fn main() {
    let mb: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(50);

    let path = std::env::temp_dir().join("hythe-term-throughput.txt");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let line = "the quick brown fox jumps over the lazy dog 0123456789 \x1b[32mgreen\x1b[0m text\r\n";
        let mut written = 0;
        while written < mb * 1024 * 1024 {
            f.write_all(line.as_bytes()).unwrap();
            written += line.len();
        }
    }
    let bytes = std::fs::metadata(&path).unwrap().len() as f64;

    // `python` as the second argument writes the file in big chunks, which
    // separates the cost of the terminal from the cost of `type`/`cat`.
    let cfg = if std::env::args().nth(2).as_deref() == Some("python") {
        let code = format!(
            "import sys\nd=open(r'{}','rb').read()\nsys.stdout.buffer.write(d)\nsys.stdout.flush()",
            path.display()
        );
        SpawnConfig::new("python").args(["-c", &code])
    } else if cfg!(windows) {
        SpawnConfig::new("cmd.exe").args(["/C", "type", &path.to_string_lossy()])
    } else {
        SpawnConfig::new("cat").args([path.to_string_lossy().to_string()])
    };

    let mut mgr = TerminalManager::default();
    mgr.set_default_size(200, 50);
    let start = Instant::now();
    let id = mgr.spawn(&cfg).unwrap();
    let s = mgr.get(id).unwrap();

    // Behave like a UI: grab a snapshot every ~16ms while output streams.
    let mut snapshots = 0u32;
    let mut snapshot_time = Duration::ZERO;
    while s.exit_code().is_none() {
        if s.take_dirty() {
            let t = Instant::now();
            let snap = s.snapshot();
            snapshot_time += t.elapsed();
            snapshots += 1;
            std::hint::black_box(snap);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let total = start.elapsed();

    println!("streamed {:.0} MB in {:.2}s = {:.0} MB/s", bytes / 1e6, total.as_secs_f64(), bytes / 1e6 / total.as_secs_f64());
    if snapshots > 0 {
        println!(
            "{} snapshots, avg {:.0} µs each (200x50 grid)",
            snapshots,
            snapshot_time.as_micros() as f64 / snapshots as f64
        );
    }
    let _ = std::fs::remove_file(path);
}
