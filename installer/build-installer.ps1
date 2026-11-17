# Builds dist\Hythe-Setup-<version>.exe: a normal Windows setup wizard (Inno Setup).
#   powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1
# Optional: -Version 0.1.13 (default: the newest git tag). Needs Rust and Inno Setup 6
# (winget install JRSoftware.InnoSetup).
param([string]$Version = "")
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not $Version) {
    $Version = (git describe --tags --abbrev=0 2>$null)
    if (-not $Version) { $Version = "0.1.0" }
}
$Version = $Version.TrimStart("v")

$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found. Install it: winget install JRSoftware.InnoSetup" }

# A separate target folder and a statically linked C runtime, so the installed app
# runs on a clean Windows without the Visual C++ redistributable. HYTHE_VERSION is what
# lets the installed app know which release it is, for self-updating.
$env:CARGO_TARGET_DIR = "target\dist"
$env:RUSTFLAGS = "-C target-feature=+crt-static"
$env:HYTHE_VERSION = $Version

cargo build --release -p hythe -p hythe-agent
if ($LASTEXITCODE -ne 0) { throw "building the app failed" }

$payload = (Resolve-Path "target\dist\release").Path
New-Item -ItemType Directory -Force dist | Out-Null
& $iscc "/DMyVersion=$Version" "/DPayload=$payload" "installer\hythe.iss"
if ($LASTEXITCODE -ne 0) { throw "building the installer failed" }

$out = "dist\Hythe-Setup-$Version.exe"
$hash = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
"$hash  Hythe-Setup-$Version.exe" | Set-Content "dist\Hythe-Setup-$Version.exe.sha256"
"{0} ({1:N1} MB)" -f $out, ((Get-Item $out).Length / 1MB)
