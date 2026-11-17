# Installing Hythe (Windows)

1. Download **Hythe-Setup-x.y.z.exe** from the [latest release](../../releases/latest).
2. Double-click it. No admin rights and no Rust needed. It installs for your user and adds a Start menu shortcut.
3. Install [Git](https://git-scm.com) if you don't have it: Hythe uses it for source control, threads and chat.
4. Optional: install the agent CLIs you want to talk to (`claude`, `copilot`, `codex`, `gemini`, ...). Hythe finds them on your PATH.

## Updates

Hythe updates itself. When you open it, it checks GitHub for a newer release (at most every 6 hours), downloads the installer, verifies its checksum and installs it quietly. The new version starts the next time you open Hythe. You never compile or reinstall by hand. To turn it off, set the `HYTHE_NO_UPDATE` environment variable. To update right now, run `Uninstall.exe /update` in the install folder, or just run the newest Hythe-Setup again.

Silent install for scripts: `Hythe-Setup.exe /S` (`/dir=PATH` to choose the folder, `/desktop` for a desktop shortcut).
Remove it from Settings > Apps, or run `Uninstall.exe` in the install folder. Your repositories and chats are never touched.

## Building the installer yourself

```powershell
powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1
```

This needs the Rust toolchain once, and writes `dist\Hythe-Setup-<version>.exe`. Pushing a tag like `v0.1.12` makes GitHub Actions build and publish it automatically.
