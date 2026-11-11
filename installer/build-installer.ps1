# Builds dist\Hythe-Setup-<version>.exe: one file that installs Hythe without Rust.
#   powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1
# Optional: -Version 0.1.12 (default: the newest git tag).
param([string]$Version = "")
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not $Version) {
    $Version = (git describe --tags --abbrev=0 2>$null)
    if (-not $Version) { $Version = "0.1.0" }
}
$Version = $Version.TrimStart("v")

# A separate target folder and a statically linked C runtime, so the installed app
# runs on a clean Windows without the Visual C++ redistributable.
$env:CARGO_TARGET_DIR = "target\dist"
$env:RUSTFLAGS = "-C target-feature=+crt-static"

cargo build --release -p hythe -p hythe-agent
if ($LASTEXITCODE -ne 0) { throw "building the app failed" }

$env:HYTHE_PAYLOAD = (Resolve-Path "target\dist\release").Path
$env:HYTHE_VERSION = $Version
cargo build --release -p hythe-installer
if ($LASTEXITCODE -ne 0) { throw "building the installer failed" }

New-Item -ItemType Directory -Force dist | Out-Null
$out = "dist\Hythe-Setup-$Version.exe"
Copy-Item "target\dist\release\hythe-installer.exe" $out -Force
$hash = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
"$hash  Hythe-Setup-$Version.exe" | Set-Content "dist\Hythe-Setup-$Version.exe.sha256"
"{0} ({1:N1} MB)" -f $out, ((Get-Item $out).Length / 1MB)
