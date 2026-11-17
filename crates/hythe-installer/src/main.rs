//! Hythe setup: one file, no admin rights, no Rust toolchain.
//!
//! Double-click to install for the current user. It copies Hythe into
//! `%LOCALAPPDATA%\Programs\Hythe`, adds a Start menu shortcut, and registers an entry in
//! Settings > Apps so it can be uninstalled there.
//!
//!   Hythe-Setup.exe                 install, asking before launching
//!   Hythe-Setup.exe /S              install silently
//!   Hythe-Setup.exe /dir=PATH       install somewhere else
//!   Hythe-Setup.exe /desktop        also put a shortcut on the desktop
//!   Hythe-Setup.exe /noshortcuts    no shortcuts (used by tests)
//!   Hythe-Setup.exe /uninstall      remove it (what Settings > Apps runs)
//!   Uninstall.exe /update           look for a newer release and install it silently
//!                                   (Hythe runs this itself on startup)

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

const HYTHE: &[u8] = include_bytes!(concat!(env!("PAYLOAD_DIR"), "/hythe.exe"));
const AGENT: &[u8] = include_bytes!(concat!(env!("PAYLOAD_DIR"), "/hythe-agent.exe"));
const VERSION: &str = match option_env!("HYTHE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
const APP: &str = "Hythe";
/// Where releases are published; the updater asks this repository for its latest one.
const REPO: &str = "git-sd/Hythe-Prototype";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Hythe";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    silent: bool,
    uninstall: bool,
    update: bool,
    desktop: bool,
    shortcuts_off: bool,
    dir: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Options {
    let mut o = Options::default();
    for a in args {
        let lower = a.to_ascii_lowercase();
        match lower.as_str() {
            "/s" | "/silent" | "--silent" => o.silent = true,
            "/uninstall" | "--uninstall" => o.uninstall = true,
            "/update" | "--update" => o.update = true,
            "/desktop" => o.desktop = true,
            "/noshortcuts" => o.shortcuts_off = true,
            _ => {
                if let Some(p) = lower.strip_prefix("/dir=") {
                    // Keep the user's own capitalisation of the path.
                    let original = &a[a.len() - p.len()..];
                    o.dir = Some(PathBuf::from(original.trim_matches('"')));
                }
            }
        }
    }
    o
}

/// A string safe inside a single-quoted PowerShell literal.
fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// "v0.1.12" -> [0, 1, 12]. Anything that is not a plain dotted number gives `None`.
fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

fn default_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"))
        .join("Programs")
        .join(APP)
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO};
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const NO_WINDOW: u32 = 0x0800_0000;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn message(text: &str, error: bool) {
        let (t, c) = (wide(text), wide(&format!("{APP} Setup")));
        let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | icon) };
    }

    pub fn ask_launch(text: &str) -> bool {
        let (t, c) = (wide(text), wide(&format!("{APP} Setup")));
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONINFORMATION) == IDYES }
    }

    fn powershell(script: &str) {
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
            .creation_flags(NO_WINDOW)
            .status();
    }

    pub fn powershell_output(script: &str) -> Result<String, String> {
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

    pub fn run_detached(exe: &Path, args: &[String]) -> Result<(), String> {
        Command::new(exe).args(args).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn start_menu() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join(format!("{APP}.lnk"))
    }

    pub fn desktop() -> PathBuf {
        std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Desktop").join(format!("{APP}.lnk"))
    }

    pub fn shortcut(lnk: &Path, target: &Path, dir: &Path) {
        if let Some(parent) = lnk.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        powershell(&format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.WorkingDirectory='{}'; $s.IconLocation='{},0'; $s.Description='{APP}'; $s.Save()",
            ps_quote(&lnk.to_string_lossy()),
            ps_quote(&target.to_string_lossy()),
            ps_quote(&dir.to_string_lossy()),
            ps_quote(&target.to_string_lossy()),
        ));
    }

    pub fn register(dir: &Path, uninstaller: &Path) -> std::io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
        key.set_value("DisplayName", &APP)?;
        key.set_value("DisplayVersion", &VERSION)?;
        key.set_value("Publisher", &APP)?;
        key.set_value("InstallLocation", &dir.to_string_lossy().to_string())?;
        key.set_value("DisplayIcon", &dir.join("hythe.exe").to_string_lossy().to_string())?;
        key.set_value("UninstallString", &format!("\"{}\" /uninstall", uninstaller.display()))?;
        key.set_value("NoModify", &1u32)?;
        key.set_value("NoRepair", &1u32)?;
        Ok(())
    }

    pub fn registered_dir() -> Option<PathBuf> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()?;
        key.get_value::<String, _>("InstallLocation").ok().map(PathBuf::from)
    }

    pub fn unregister() {
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
    }

    pub fn git_installed() -> bool {
        Command::new("git").arg("--version").creation_flags(NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false)
    }

    pub fn launch(exe: &Path) {
        let _ = Command::new(exe).current_dir(exe.parent().unwrap_or(Path::new("."))).creation_flags(NO_WINDOW).spawn();
    }

    /// Delete the install folder after this program has exited (it is running from inside it).
    pub fn remove_dir_later(dir: &Path) {
        // `raw_arg`: Rust would escape the quotes in a normal argument and cmd would not understand.
        let _ = Command::new("cmd")
            .raw_arg(format!("/C ping -n 3 127.0.0.1 >nul & rmdir /S /Q \"{}\"", dir.display()))
            .creation_flags(NO_WINDOW)
            .spawn();
    }
}

/// Replace `dest`. Windows will not overwrite a running program but will rename it, so a
/// busy file is moved aside to `.old` (removed on the next run) and the new one put in
/// its place: an update can land while Hythe is open and starts with the next launch.
#[cfg(windows)]
fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = dest.with_extension("new");
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not write {}: {e}", dest.display()))?;
    if std::fs::rename(&tmp, dest).is_ok() {
        return Ok(());
    }
    let old = dest.with_extension("old");
    let _ = std::fs::remove_file(&old);
    let result = std::fs::rename(dest, &old).and_then(|_| {
        std::fs::rename(&tmp, dest).inspect_err(|_| {
            let _ = std::fs::rename(&old, dest);
        })
    });
    result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace {}. Is {APP} still running? Close it and run setup again. ({e})", dest.display())
    })
}

#[cfg(windows)]
fn remove_leftovers(dir: &Path) {
    for f in ["hythe.old", "hythe-agent.old", "Uninstall.old"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
}

/// Ask GitHub for the newest release; if it is newer than this build, download its setup,
/// check it against the published checksum and run it silently. Returns a short note.
#[cfg(windows)]
fn update(o: &Options) -> Result<String, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = o.dir.clone().or_else(|| me.parent().map(Path::to_path_buf)).ok_or("could not find the install folder")?;
    let script = format!(
        "$ErrorActionPreference='Stop';[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocol]::Tls12;\
         $r=Invoke-RestMethod -UseBasicParsing -Headers @{{'User-Agent'='Hythe-Updater'}} 'https://api.github.com/repos/{REPO}/releases/latest';\
         $r.tag_name;foreach($a in $r.assets){{$a.browser_download_url}}"
    );
    let out = win::powershell_output(&script)?;
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let tag = lines.next().ok_or("no release found")?.to_string();
    if !is_newer(&tag, VERSION) {
        return Ok(format!("{APP} {VERSION} is up to date."));
    }
    let urls: Vec<&str> = lines.collect();
    let setup = urls.iter().find(|u| u.contains("Hythe-Setup-") && u.ends_with(".exe")).ok_or("release has no installer")?;
    let sum = urls.iter().find(|u| u.ends_with(".exe.sha256")).ok_or("release has no checksum")?;
    let tmp = std::env::temp_dir().join("hythe-update");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let file = tmp.join("Hythe-Setup.exe");
    win::powershell_output(&format!(
        "$ErrorActionPreference='Stop';[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocol]::Tls12;\
         Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{}';\
         $want=((Invoke-WebRequest -UseBasicParsing -Uri '{}').Content -split '\\s+')[0].Trim().ToLower();\
         $got=(Get-FileHash '{}' -Algorithm SHA256).Hash.ToLower();\
         if($want -ne $got){{throw 'checksum mismatch'}}",
        ps_quote(setup),
        ps_quote(&file.to_string_lossy()),
        ps_quote(sum),
        ps_quote(&file.to_string_lossy()),
    ))?;
    win::run_detached(&file, &["/S".into(), "/update".into(), format!("/dir={}", dir.display())])?;
    Ok(format!("Updating {APP} to {tag}."))
}

#[cfg(windows)]
fn install(o: &Options) -> Result<String, String> {
    if HYTHE.is_empty() || AGENT.is_empty() {
        return Err("This setup was built without the app inside it. Build it with installer\\build-installer.ps1.".into());
    }
    let dir = o.dir.clone().unwrap_or_else(default_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    remove_leftovers(&dir);
    let exe = dir.join("hythe.exe");
    write_atomic(&exe, HYTHE)?;
    write_atomic(&dir.join("hythe-agent.exe"), AGENT)?;
    // The uninstaller is this very program, kept next to the app.
    let uninstaller = dir.join("Uninstall.exe");
    if let Ok(me) = std::env::current_exe() {
        if me != uninstaller {
            write_atomic(&uninstaller, &std::fs::read(&me).map_err(|e| e.to_string())?)?;
        }
    }
    if !o.shortcuts_off && !o.update {
        win::shortcut(&win::start_menu(), &exe, &dir);
        if o.desktop {
            win::shortcut(&win::desktop(), &exe, &dir);
        }
    }
    win::register(&dir, &uninstaller).map_err(|e| format!("could not register the uninstaller: {e}"))?;
    let mut note = format!("{APP} {VERSION} is installed in {}.", dir.display());
    if !o.update && !win::git_installed() {
        note.push_str("\n\nGit was not found. Hythe uses it for source control, threads and chat: install it from https://git-scm.com and reopen Hythe.");
    }
    Ok(note)
}

#[cfg(windows)]
fn uninstall(o: &Options) -> Result<String, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = o
        .dir
        .clone()
        .or_else(win::registered_dir)
        .or_else(|| me.parent().map(Path::to_path_buf))
        .ok_or("could not find where Hythe is installed")?;
    let _ = std::fs::remove_file(win::start_menu());
    let _ = std::fs::remove_file(win::desktop());
    win::unregister();
    for f in ["hythe.exe", "hythe-agent.exe"] {
        std::fs::remove_file(dir.join(f)).map_err(|e| format!("could not remove {f}. Is {APP} still running? ({e})"))?;
    }
    // Uninstall.exe is running right now, so the folder goes once it has exited.
    win::remove_dir_later(&dir);
    Ok(format!("{APP} was removed. Your repositories and chats are untouched."))
}

/// Both the installed `Uninstall.exe` and a freshly downloaded setup contain the app, so
/// the two `/update` roles are told apart by the file name: only `Uninstall.exe` looks
/// for a newer release.
#[cfg(windows)]
fn is_installed_updater() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_ascii_lowercase()))
        .is_some_and(|n| n == "uninstall.exe")
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = parse_args(&args);
    // `/update` from the installed Uninstall.exe fetches and runs the newest setup; the
    // downloaded setup then runs `/update` too, but with the app inside it, and installs.
    let result = if o.uninstall {
        uninstall(&o)
    } else if o.update && is_installed_updater() {
        update(&o)
    } else {
        install(&o)
    };
    match result {
        Ok(note) => {
            if o.silent {
                return;
            }
            if o.uninstall {
                win::message(&note, false);
            } else if win::ask_launch(&format!("{note}\n\nOpen {APP} now?")) {
                win::launch(&o.dir.clone().unwrap_or_else(default_dir).join("hythe.exe"));
            }
        }
        Err(e) => {
            if !o.silent {
                win::message(&e, true);
            }
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    let _ = (HYTHE.len(), AGENT.len(), VERSION, default_dir());
    eprintln!("The {APP} installer is for Windows.");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn options_are_read_from_the_command_line() {
        let o = parse_args(&args(&["/S", "/DIR=C:\\Tools\\Hythe", "/desktop"]));
        assert!(o.silent && o.desktop && !o.uninstall);
        assert_eq!(o.dir, Some(PathBuf::from("C:\\Tools\\Hythe")), "the path keeps its capitalisation");
        assert!(parse_args(&args(&["/uninstall"])).uninstall);
        assert!(parse_args(&args(&["/update", "/S"])).update);
        assert_eq!(parse_args(&args(&[])), Options::default());
        let q = parse_args(&args(&["/dir=\"C:\\Program Files\\Hythe\""]));
        assert_eq!(q.dir, Some(PathBuf::from("C:\\Program Files\\Hythe")));
    }

    #[test]
    fn powershell_literals_cannot_be_broken_out_of() {
        assert_eq!(ps_quote("C:\\Users\\O'Brien\\Hythe"), "C:\\Users\\O''Brien\\Hythe");
    }

    #[test]
    fn only_a_higher_release_counts_as_an_update() {
        assert!(is_newer("v0.1.13", "0.1.12"));
        assert!(is_newer("v0.2.0", "0.1.12"));
        assert!(is_newer("v0.1.12", "0.1.9"), "numbers, not text");
        assert!(!is_newer("v0.1.12", "0.1.12"));
        assert!(!is_newer("v0.1.11", "0.1.12"));
        assert!(!is_newer("nightly", "0.1.12"), "an odd tag is never installed");
    }

    #[test]
    fn the_default_folder_is_per_user() {
        assert!(default_dir().ends_with("Programs/Hythe") || default_dir().ends_with("Programs\\Hythe"));
    }
}
