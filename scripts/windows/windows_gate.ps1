$ErrorActionPreference = "Stop"

function Fail([string]$msg) {
  Write-Host ("FAIL: " + $msg) -ForegroundColor Red
  exit 1
}

function Info([string]$msg) {
  Write-Host ("INFO: " + $msg) -ForegroundColor Cyan
}

function Run-Native([string]$label, [string]$exe, [string[]]$cmdArgs) {
  Info $label
  & $exe @cmdArgs | Out-Host
  if ($LASTEXITCODE -ne 0) {
    Fail ($label + " failed (exit_code=" + $LASTEXITCODE + ")")
  }
}

function Ensure-Command([string]$name) {
  if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
    Fail ("missing command: " + $name)
  }
}

function Try-Enable-Sccache([string]$repoRoot) {
  if (Get-Command "sccache" -ErrorAction SilentlyContinue) {
    $env:RUSTC_WRAPPER = "sccache"
    $env:SCCACHE_DIR = (Join-Path $repoRoot ".cache\\sccache")
    New-Item -ItemType Directory -Force -Path $env:SCCACHE_DIR | Out-Null
    Info ("sccache enabled: RUSTC_WRAPPER=sccache SCCACHE_DIR=" + $env:SCCACHE_DIR)
  } else {
    Info "sccache not found (optional). Install once with: cargo install sccache"
  }
}

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $RepoRoot

Info ("repo_root=" + $RepoRoot)

# Basic toolchain checks (we don't auto-install global tools; just fail with a clear message)
Ensure-Command "node"
Ensure-Command "npm"
Ensure-Command "cargo"
Ensure-Command "gpg"

# Fast fail: Windows-only compile gate (catches Send/Sync and other Windows-specific errors).
Run-Native "running windows compile gate" "powershell" @(
  "-ExecutionPolicy",
  "Bypass",
  "-File",
  ".\scripts\windows\windows_compile_gate.ps1"
)

# Optional speed-up: enable Rust compile cache if available.
Try-Enable-Sccache $RepoRoot

Run-Native "downloading bundled ffmpeg toolchain" "powershell" @(
  "-ExecutionPolicy",
  "Bypass",
  "-File",
  ".\scripts\windows\download_ffmpeg_toolchain.ps1",
  "-Platform",
  "all"
)

$ToolchainDir = Join-Path $RepoRoot "apps\desktop\src-tauri\toolchain\bin\windows-x86_64"
$env:TYPEVOICE_TOOLCHAIN_DIR = $ToolchainDir
$env:TYPEVOICE_FFMPEG = (Join-Path $ToolchainDir "ffmpeg.exe")
$env:TYPEVOICE_FFPROBE = (Join-Path $ToolchainDir "ffprobe.exe")

# Desktop deps
Set-Location (Join-Path $RepoRoot "apps\desktop")
Run-Native "installing desktop npm deps" "npm" @("ci")

Run-Native "running desktop build" "npm" @("run", "build")

Set-Location (Join-Path $RepoRoot "apps\desktop\src-tauri")
Run-Native "running rust tests" "cargo" @("test", "--locked")

Info "starting desktop app (tauri dev)"
Set-Location (Join-Path $RepoRoot "apps\desktop")
npm run tauri dev
