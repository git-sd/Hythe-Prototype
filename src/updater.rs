//! Keeps the installed Hythe current. On startup this asks the installer that sits next
//! to `hythe.exe` (`Uninstall.exe`) to look for a newer GitHub release and apply it
//! silently. The new version takes effect the next time Hythe is opened.
//!
//! Does nothing for a `cargo run` build (there is no installer beside it), and can be
//! switched off with the `HYTHE_NO_UPDATE` environment variable.

use std::time::{Duration, SystemTime};

/// At most one check per this long, so opening Hythe often doesn't hit GitHub each time.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

pub fn check_in_background() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if std::env::var_os("HYTHE_NO_UPDATE").is_some() {
            return;
        }
        let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) else { return };
        let updater = dir.join("Uninstall.exe");
        if !updater.is_file() {
            return;
        }
        let stamp = dir.join("update-check.stamp");
        let recent = std::fs::metadata(&stamp)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < EVERY);
        if recent {
            return;
        }
        let _ = std::fs::write(&stamp, b"");
        // Detached and windowless: it must outlive nothing and show nothing.
        let _ = std::process::Command::new(updater)
            .args(["/update", "/S"])
            .current_dir(&dir)
            .creation_flags(0x0800_0000)
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = EVERY;
}
