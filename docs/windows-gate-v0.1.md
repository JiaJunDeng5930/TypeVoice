# Windows Gate v0.1

This repo is primarily developed in WSL, but the target runtime for the MVP is Windows desktop.

## One-Command Gate (PowerShell)

Run in a Windows PowerShell terminal (non-admin is fine):

```powershell
Set-Location C:\path\to\TypeVoice
powershell -ExecutionPolicy Bypass -File .\scripts\windows\windows_gate.ps1
```

What it does:

- Creates repo-local venv (`.venv`) and installs Python deps (including Torch CUDA wheels).
- Downloads ASR model into `models/Qwen3-ASR-0.6B` (gitignored).
- Installs `apps/desktop` npm deps via `npm ci`.
- Runs `verify_quick` and `verify_full`.
- Starts `npm run tauri dev`.

## Expected Failures

The script intentionally fails fast if any of these are missing:

- `python`, `node`, `npm`, `cargo`, `ffmpeg`, `ffprobe` are not in PATH.
- `torch.cuda.is_available()` is `False` after installing Torch wheels.

