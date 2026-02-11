$ErrorActionPreference = "Stop"

function Fail([string]$msg) {
  Write-Host ("FAIL: " + $msg) -ForegroundColor Red
  exit 1
}

function Info([string]$msg) {
  Write-Host ("INFO: " + $msg) -ForegroundColor Cyan
}

function Ensure-Command([string]$name) {
  if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
    Fail ("missing command: " + $name)
  }
}

function Ensure-CargoInPath {
  if (Get-Command "cargo" -ErrorAction SilentlyContinue) {
    return
  }
  $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
  $cargoExe = Join-Path $cargoBin "cargo.exe"
  if (Test-Path $cargoExe) {
    $env:PATH = $cargoBin + ";" + $env:PATH
  }
}

# This script intentionally assumes nothing about where the repo lives.
# The caller must run it from the repo root.
if (-not (Test-Path "apps/desktop/src-tauri/Cargo.toml")) {
  Fail "run this script from the repo root (expected apps/desktop/src-tauri/Cargo.toml)"
}

Ensure-CargoInPath
Ensure-Command "cargo"

Info "running: cargo check (Windows compile gate)"
Push-Location "apps/desktop/src-tauri"
try {
  # On UNC paths (e.g. \\wsl.localhost\...), Rust incremental lock files may fail on Windows.
  # Use a local target dir and disable incremental mode for deterministic compile-gate behavior.
  if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:TEMP "typevoice-target"
  }
  $env:CARGO_INCREMENTAL = "0"
  New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null
  cargo check --locked
  if ($LASTEXITCODE -ne 0) {
    Fail ("cargo check failed (exit_code=" + $LASTEXITCODE + ")")
  }
} finally {
  Pop-Location
}

Write-Host "PASS" -ForegroundColor Green
exit 0
