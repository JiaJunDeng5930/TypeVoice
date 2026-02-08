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

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $RepoRoot

Info ("repo_root=" + $RepoRoot)

# Basic toolchain checks (we don't auto-install global tools; just fail with a clear message)
Ensure-Command "python"
Ensure-Command "node"
Ensure-Command "npm"
Ensure-Command "cargo"
Ensure-Command "ffmpeg"
Ensure-Command "ffprobe"

# Python venv (repo-local)
$VenvPython = Join-Path $RepoRoot ".venv\Scripts\python.exe"
if (-not (Test-Path $VenvPython)) {
  Info "creating .venv"
  python -m venv .venv | Out-Host
}

& $VenvPython -m pip install -U pip | Out-Host

# Torch CUDA install (try several common CUDA wheel indexes; fail if CUDA still unavailable)
$TorchIndexUrls = @(
  "https://download.pytorch.org/whl/cu126",
  "https://download.pytorch.org/whl/cu124",
  "https://download.pytorch.org/whl/cu121"
)

$TorchOk = $false
foreach ($url in $TorchIndexUrls) {
  Info ("installing torch from " + $url)
  try {
    & $VenvPython -m pip install -U torch torchvision torchaudio --index-url $url | Out-Host
    $cuda = & $VenvPython -c "import torch; print('cuda_available=' + str(torch.cuda.is_available()))"
    if ($cuda -match "cuda_available=True") {
      $TorchOk = $true
      break
    }
  } catch {
    # keep trying
  }
}

if (-not $TorchOk) {
  Fail "torch CUDA is not available (torch.cuda.is_available() == False). Check NVIDIA driver / GPU, then re-run."
}

Info "installing python deps"
& $VenvPython -m pip install -U huggingface_hub pytest qwen-asr transformers accelerate | Out-Host

# Download model (repo-local, ignored by git)
Info "downloading ASR model"
& $VenvPython scripts\download_asr_model.py | Out-Host

# Desktop deps
Info "installing desktop npm deps"
Set-Location (Join-Path $RepoRoot "apps\desktop")
npm ci | Out-Host

# Gate: quick/full (machine-readable metrics are written to metrics/verify.jsonl)
Set-Location $RepoRoot
Info "running verify_quick"
& $VenvPython scripts\verify_quick.py | Out-Host

Info "running verify_full"
& $VenvPython scripts\verify_full.py | Out-Host

Info "starting desktop app (tauri dev)"
Set-Location (Join-Path $RepoRoot "apps\desktop")
npm run tauri dev

