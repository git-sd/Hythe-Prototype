# Installing Hythe (Windows)

1. Download **Hythe-Setup-x.y.z.exe** from the [latest release](../../releases/latest).
2. Double-click it and follow the wizard (Next, Install, Finish). No admin rights and no Rust needed. It installs for your user and adds a Start menu shortcut. Windows may show a "protected your PC" warning the first time because the installer isn't code-signed: choose **More info > Run anyway**.
3. Install [Git](https://git-scm.com) if you don't have it: Hythe uses it for source control, threads and chat.
4. Optional: install the agent CLIs you want to talk to (`claude`, `copilot`, `codex`, `gemini`, ...). Hythe finds them on your PATH.

## Updates

Hythe updates itself. While it runs it checks GitHub for a newer release (at most every 6 hours), downloads the installer and verifies its checksum. The next time you open Hythe it installs the update in a couple of seconds and reopens. You never compile or reinstall by hand. To turn it off, set the `HYTHE_NO_UPDATE` environment variable.

Silent install for scripts: `Hythe-Setup.exe /VERYSILENT` (`/DIR=PATH` to choose the folder, `/TASKS=desktopicon` for a desktop shortcut).
Remove it from Settings > Apps. Your repositories and chats are never touched.

## Building the installer yourself

```powershell
powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1
```

This needs the Rust toolchain and [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`), and writes `dist\Hythe-Setup-<version>.exe`. Pushing a tag like `v0.1.14` makes GitHub Actions build and publish it automatically.
