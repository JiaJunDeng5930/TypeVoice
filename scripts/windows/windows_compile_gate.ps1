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

# This script intentionally assumes nothing about where the repo lives.
# The caller must run it from the repo root.
if (-not (Test-Path "apps/desktop/src-tauri/Cargo.toml")) {
  Fail "run this script from the repo root (expected apps/desktop/src-tauri/Cargo.toml)"
}

Ensure-Command "cargo"

Info "running: cargo check (Windows compile gate)"
Push-Location "apps/desktop/src-tauri"
try {
  cargo check --locked
} finally {
  Pop-Location
}

Write-Host "PASS" -ForegroundColor Green
exit 0

