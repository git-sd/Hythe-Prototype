//! Keeps the installed Hythe current.
//!
//! While Hythe runs, a background thread asks GitHub for the newest release. If it is
//! newer than this build, the setup is downloaded, checked against the published
//! checksum and staged in `<install folder>\update`. The next time Hythe starts it hands
//! that setup the job (silently) and exits; the setup replaces the files and reopens Hythe.
//!
//! Only installed builds take part: `HYTHE_VERSION` is set when the installer is built,
//! so a `cargo run` build never updates itself. `HYTHE_NO_UPDATE` switches it off.

const REPO: &str = "git-sd/Hythe-Prototype";

/// "v0.1.12" -> [0, 1, 12]. Anything that is not a plain dotted number gives `None`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{Duration, SystemTime};

    const NO_WINDOW: u32 = 0x0800_0000;
    /// At most one check per this long.
    const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

    fn enabled() -> Option<&'static str> {
        if std::env::var_os("HYTHE_NO_UPDATE").is_some() {
            return None;
        }
        option_env!("HYTHE_VERSION")
    }

    fn install_dir() -> Option<PathBuf> {
        std::env::current_exe().ok()?.parent().map(PathBuf::from)
    }

    fn powershell(script: &str) -> Result<String, String> {
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
            .creation_flags(NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    fn quote(s: &str) -> String {
        s.replace('\'', "''")
    }

    /// If a downloaded update is waiting, run it and return true: the caller should exit
    /// at once so the setup can replace the program.
    pub fn apply_staged() -> bool {
        if enabled().is_none() {
            return false;
        }
        let Some(dir) = install_dir() else { return false };
        let staged = dir.join("update").join("Hythe-Setup.exe");
        if !staged.is_file() {
            return false;
        }
        // Run it from the temp folder (it must not sit in the folder it is replacing) and
        // only once: a setup that fails does not get retried on every launch.
        let run = std::env::temp_dir().join("Hythe-Update-Setup.exe");
        let _ = std::fs::remove_file(&run);
        if std::fs::rename(&staged, &run).is_err() && std::fs::copy(&staged, &run).is_err() {
            return false;
        }
        let _ = std::fs::remove_file(&staged);
        Command::new(&run)
            .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/RELAUNCH=1"])
            .creation_flags(NO_WINDOW)
            .spawn()
            .is_ok()
    }

    pub fn check_in_background() {
        let Some(current) = enabled() else { return };
        let Some(dir) = install_dir() else { return };
        let stamp = dir.join("update-check.stamp");
        let recent = std::fs::metadata(&stamp)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < EVERY);
        if recent {
            return;
        }
        std::thread::spawn(move || {
            let _ = std::fs::write(&stamp, b"");
            let _ = stage_latest(&dir, current);
        });
    }

    fn stage_latest(dir: &std::path::Path, current: &str) -> Result<(), String> {
        let tls = "$ErrorActionPreference='Stop';[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocol]::Tls12;";
        let out = powershell(&format!(
            "{tls}$r=Invoke-RestMethod -UseBasicParsing -Headers @{{'User-Agent'='Hythe-Updater'}} 'https://api.github.com/repos/{REPO}/releases/latest';$r.tag_name;foreach($a in $r.assets){{$a.browser_download_url}}"
        ))?;
        let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
        let tag = lines.next().ok_or("no release found")?;
        if !is_newer(tag, current) {
            return Ok(());
        }
        let urls: Vec<&str> = lines.collect();
        let setup = urls.iter().find(|u| u.contains("Hythe-Setup-") && u.ends_with(".exe")).ok_or("no installer in release")?;
        let sum = urls.iter().find(|u| u.ends_with(".exe.sha256")).ok_or("no checksum in release")?;
        let folder = dir.join("update");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let partial = folder.join("download.tmp");
        let ready = folder.join("Hythe-Setup.exe");
        powershell(&format!(
            "{tls}Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{}';\
             $want=((Invoke-WebRequest -UseBasicParsing -Uri '{}').Content -split '\\s+')[0].Trim().ToLower();\
             $got=(Get-FileHash '{}' -Algorithm SHA256).Hash.ToLower();\
             if($want -ne $got){{Remove-Item '{}';throw 'checksum mismatch'}}",
            quote(setup),
            quote(&partial.to_string_lossy()),
            quote(sum),
            quote(&partial.to_string_lossy()),
            quote(&partial.to_string_lossy()),
        ))?;
        // Renamed last, so a half-downloaded file is never mistaken for a ready update.
        std::fs::rename(&partial, &ready).map_err(|e| e.to_string())
    }
}

/// Run a downloaded update if one is waiting. Returns true when the caller must exit.
pub fn apply_staged() -> bool {
    #[cfg(windows)]
    return imp::apply_staged();
    #[cfg(not(windows))]
    false
}

pub fn check_in_background() {
    #[cfg(windows)]
    imp::check_in_background();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_higher_release_counts_as_an_update() {
        assert!(is_newer("v0.1.13", "0.1.12"));
        assert!(is_newer("v0.2.0", "0.1.12"));
        assert!(is_newer("v0.1.12", "0.1.9"), "numbers, not text");
        assert!(!is_newer("v0.1.12", "0.1.12"));
        assert!(!is_newer("v0.1.11", "0.1.12"));
        assert!(!is_newer("nightly", "0.1.12"), "an odd tag is never installed");
    }
}
