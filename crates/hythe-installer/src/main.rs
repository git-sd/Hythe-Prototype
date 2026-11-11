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

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

const HYTHE: &[u8] = include_bytes!(concat!(env!("PAYLOAD_DIR"), "/hythe.exe"));
const AGENT: &[u8] = include_bytes!(concat!(env!("PAYLOAD_DIR"), "/hythe-agent.exe"));
const VERSION: &str = match option_env!("HYTHE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
const APP: &str = "Hythe";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Hythe";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    silent: bool,
    uninstall: bool,
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
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.WorkingDirectory='{}'; $s.Description='{APP}'; $s.Save()",
            ps_quote(&lnk.to_string_lossy()),
            ps_quote(&target.to_string_lossy()),
            ps_quote(&dir.to_string_lossy()),
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

#[cfg(windows)]
fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = dest.with_extension("new");
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not write {}: {e}", dest.display()))?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace {}. Is {APP} still running? Close it and run setup again. ({e})", dest.display())
    })
}

#[cfg(windows)]
fn install(o: &Options) -> Result<String, String> {
    if HYTHE.is_empty() || AGENT.is_empty() {
        return Err("This setup was built without the app inside it. Build it with installer\\build-installer.ps1.".into());
    }
    let dir = o.dir.clone().unwrap_or_else(default_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
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
    if !o.shortcuts_off {
        win::shortcut(&win::start_menu(), &exe, &dir);
        if o.desktop {
            win::shortcut(&win::desktop(), &exe, &dir);
        }
    }
    win::register(&dir, &uninstaller).map_err(|e| format!("could not register the uninstaller: {e}"))?;
    let mut note = format!("{APP} {VERSION} is installed in {}.", dir.display());
    if !win::git_installed() {
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

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = parse_args(&args);
    let result = if o.uninstall { uninstall(&o) } else { install(&o) };
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
        assert_eq!(parse_args(&args(&[])), Options::default());
        let q = parse_args(&args(&["/dir=\"C:\\Program Files\\Hythe\""]));
        assert_eq!(q.dir, Some(PathBuf::from("C:\\Program Files\\Hythe")));
    }

    #[test]
    fn powershell_literals_cannot_be_broken_out_of() {
        assert_eq!(ps_quote("C:\\Users\\O'Brien\\Hythe"), "C:\\Users\\O''Brien\\Hythe");
    }

    #[test]
    fn the_default_folder_is_per_user() {
        assert!(default_dir().ends_with("Programs/Hythe") || default_dir().ends_with("Programs\\Hythe"));
    }
}
