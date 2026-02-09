# Windows Dev From WSL v0.1

Goal: reliably build and run the Windows desktop app while keeping the main development workflow inside WSL.

This doc records the exact method we used during debugging sessions, so it can be repeated without relying on memory.

## 0. Mental Model

There are two separate environments:

- WSL filesystem + Linux toolchain (primary editing happens here).
- Windows runtime + Windows toolchain (the MVP runs here).

Tauri dev must run with the Windows toolchain to launch a real Windows `typevoice-desktop.exe`.

## 1. Repo Layout (Two Working Copies)

Keep a Windows-side working copy (recommended path uses `D:` to avoid `C:` space pressure):

- Windows repo: `D:\Projects\TypeVoice`
- WSL repo: `/home/atticusdeng/Projects/TypeVoice`

Do not run Windows builds from a UNC path like `\\wsl.localhost\...` (it causes `cmd.exe` issues and is fragile).

## 2. How Windows Repo Syncs From WSL Repo

The Windows repo uses the WSL repo as a Git remote (via UNC path).

Verify in Windows:

```powershell
Set-Location D:\Projects\TypeVoice
git remote -v
```

Expected: `origin` (or another remote) points to something like:

- `\\wsl.localhost\Ubuntu-24.04\home\atticusdeng\Projects\TypeVoice`

Sync changes (Windows repo pulls from WSL repo):

```powershell
Set-Location D:\Projects\TypeVoice
git pull
```

Notes:

- If `git pull` complains about local changes, reset only the impacted file(s) you did not intend to modify:
  - Example: `git checkout -- apps/desktop/src-tauri/Cargo.toml`
- Line ending warnings (`LF will be replaced by CRLF`) can appear on Windows. Avoid manual edits in Windows unless needed.

## 3. Run The App On Windows (Direct Windows Terminal)

Run from Windows PowerShell or Windows Terminal (non-admin is fine):

```powershell
Set-Location D:\Projects\TypeVoice\apps\desktop
$env:RUST_BACKTRACE = "1"
$env:RUST_LOG = "debug"
# Optional speed-up (install once: `cargo install sccache`)
# $env:RUSTC_WRAPPER = "sccache"
npm run tauri dev
```

What happens:

- Starts Vite (`npm run dev`) for the frontend.
- Runs `cargo run` for the Tauri Rust backend.
- Launches `target\debug\typevoice-desktop.exe`.

## 4. Run The App On Windows (Triggered From Inside WSL)

This is the exact pattern used in debugging sessions: WSL calls Windows shells and runs the Windows toolchain.

### 4.1 Start dev (interactive console)

From WSL:

```bash
/mnt/c/Windows/System32/cmd.exe /d /c "cd /d D:\Projects\TypeVoice\apps\desktop && set RUST_BACKTRACE=1 && set RUST_LOG=debug && npm run tauri dev"
```

Optional speed-up (`sccache`, install once in Windows: `cargo install sccache`):

```bash
/mnt/c/Windows/System32/cmd.exe /d /c "cd /d D:\Projects\TypeVoice\apps\desktop && set RUST_BACKTRACE=1 && set RUST_LOG=debug && set RUSTC_WRAPPER=sccache && npm run tauri dev"
```

Why `cd /d`:

- Windows `cmd.exe` must switch drive (`D:`) explicitly.
- It avoids UNC cwd issues.

### 4.2 Start dev and write logs to D:

From WSL:

```bash
/mnt/c/Windows/System32/cmd.exe /d /c "cd /d D:\Projects\TypeVoice\apps\desktop && set RUST_BACKTRACE=1 && set RUST_LOG=debug && npm run tauri dev > D:\typevoice-logs\tauri-dev-run.txt 2>&1"
```

Read the log from WSL:

```bash
tail -n 200 /mnt/d/typevoice-logs/tauri-dev-run.txt
```

This is the recommended method when chasing crashes, because the dev console can disappear when the process aborts.

## 5. Check Whether The App Is Running

In Windows (PowerShell):

```powershell
Get-Process -Name typevoice-desktop -ErrorAction SilentlyContinue |
  Select-Object -First 1 |
  Format-List -Property Id,StartTime,Responding
```

From WSL, you can invoke the same command:

```bash
/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Get-Process -Name typevoice-desktop -ErrorAction SilentlyContinue | Select-Object -First 1 | Format-List -Property Id,StartTime,Responding"
```

## 6. Stop All Dev Processes (Windows)

Useful when Vite/Node/Rust watchers get stuck.

```powershell
taskkill /IM typevoice-desktop.exe /F
taskkill /IM node.exe /F
```

From WSL:

```bash
/mnt/c/Windows/System32/cmd.exe /d /c "cd /d D:\ && taskkill /IM typevoice-desktop.exe /F >nul 2>&1 & taskkill /IM node.exe /F >nul 2>&1 & exit /b 0"
```

## 7. Common Pitfalls And Fixes

### 7.1 `cmd.exe` says UNC paths are not supported

Symptom:

- `CMD.EXE was started with the above path as the current directory. UNC paths are not supported.`

Fix:

- Always start Windows commands with `cd /d D:\...` (never rely on inherited cwd from WSL).

### 7.2 "Works in WSL but not in Windows"

Root cause:

- Different toolchains and different PATH resolution.

Fix:

- Treat Windows as the source of truth for runtime: always run `npm run tauri dev` in the Windows repo.

### 7.3 Dev console disappears after crash

Fix:

- Use file redirection to `D:\typevoice-logs\...` and inspect logs from WSL via `/mnt/d/...`.
