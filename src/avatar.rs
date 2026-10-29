//! Downloads and caches a GitHub profile picture for the account chip.
//!
//! The picture is fetched from the public URL `https://github.com/<user>.png`
//! with `curl` (present on current Windows, macOS and Linux), so no HTTP client
//! is linked into the app. Only the username is sent, and the file is cached for
//! a week under the app's data folder.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("hythe").join("avatars"))
}

/// PNG, JPEG or GIF: what GitHub serves for avatars.
pub fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G']) || bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(b"GIF8")
}

fn is_valid_image(path: &Path) -> bool {
    std::fs::read(path).map(|b| b.len() > 100 && looks_like_image(&b)).unwrap_or(false)
}

fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < MAX_AGE)
}

/// The profile picture for `user`: from the cache if recent, otherwise downloaded.
/// Returns `None` if there's no picture and no network.
pub fn fetch(user: &str) -> Option<PathBuf> {
    if !hythe_git::is_github_login(user) {
        return None;
    }
    download(user, &format!("https://github.com/{user}.png?size=96"))
}

fn download(stem: &str, url: &str) -> Option<PathBuf> {
    let dir = cache_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("{stem}.png"));
    if is_valid_image(&file) && is_fresh(&file) {
        return Some(file);
    }

    let tmp = dir.join(format!("{stem}.download"));
    let mut cmd = Command::new("curl");
    cmd.args(["-sSL", "-m", "10", "-A", "Mozilla/5.0", "-o"]).arg(&tmp).arg(url);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok && is_valid_image(&tmp) && std::fs::rename(&tmp, &file).is_ok() {
        return Some(file);
    }
    let _ = std::fs::remove_file(&tmp);
    // Offline: an old picture beats a letter.
    is_valid_image(&file).then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_image_headers() {
        assert!(looks_like_image(&[0x89, b'P', b'N', b'G', 0, 0]));
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(looks_like_image(b"GIF89a"));
        assert!(!looks_like_image(b"<html>not found"));
        assert!(!looks_like_image(b""));
    }

    #[test]
    fn invalid_names_are_never_fetched() {
        assert_eq!(fetch("Shreyan Das"), None);
        assert_eq!(fetch("../etc"), None);
        assert_eq!(fetch(""), None);
    }
}
