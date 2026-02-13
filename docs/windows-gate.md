# Windows Gate

This repo is primarily developed in WSL, but the target runtime for the MVP is Windows desktop.

For a detailed, manual, repeatable workflow (including how WSL triggers Windows builds), see:

- `docs/windows-dev.md`

## One-Command Gate (PowerShell)

Run in a Windows PowerShell terminal (non-admin is fine):

```powershell
Set-Location C:\path\to\TypeVoice
powershell -ExecutionPolicy Bypass -File .\scripts\windows\windows_gate.ps1
```

For day-to-day latest-code launch/debug, use:

```powershell
Set-Location D:\Projects\TypeVoice
.\scripts\windows\run-latest.ps1
```

该脚本会先下载并校验受控 FFmpeg 工具链（`apps/desktop/src-tauri/toolchain/bin/...`），不再依赖系统 PATH 里的 `ffmpeg/ffprobe`。

## Fast Gate: Windows Compile Check

If you only need a quick "does the Windows Rust backend compile?" check, run:

```powershell
Set-Location C:\path\to\TypeVoice
powershell -ExecutionPolicy Bypass -File .\scripts\windows\windows_compile_gate.ps1
```

What it does:

- Creates repo-local venv (`.venv`) and installs Python deps (including Torch CUDA wheels).
- Downloads ASR model into `models/Qwen3-ASR-0.6B` (gitignored).
- Installs `apps/desktop` npm deps via `npm ci`.
- Runs `verify_quick` and `verify_full`.
- Starts `npm run tauri dev`.

## Optional Speed-Up: sccache (Rust Compile Cache)

If `sccache` is installed and available in PATH, the gate script will automatically enable it for Rust builds.

Install once (Windows PowerShell):

```powershell
cargo install sccache
```

## Expected Failures

The script intentionally fails fast if any of these are missing:

- `python`, `node`, `npm`, `cargo` are not in PATH.
- bundled FFmpeg toolchain download/checksum validation fails.
- `torch.cuda.is_available()` is `False` after installing Torch wheels.
