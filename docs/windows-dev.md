# Windows Dev From WSL

Goal: reliably build and run the Windows desktop app while keeping the main development workflow inside WSL.

## 0. Authoritative One-Command Run

Use this command whenever you want to launch the Windows runtime from the latest code.

From Windows PowerShell:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask run latest
```

From WSL, invoke the same Windows runtime command:

```bash
/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Set-Location D:\Projects\TypeVoice; cargo xtask run latest"
```

This command:

- stops stale `typevoice-desktop.exe` and repo-related `node.exe` processes;
- runs `npm ci`;
- runs `npm run build`;
- starts `npm run tauri dev`;
- prints PID, executable path, log path, and latest log tail.

If the command fails, read:

```bash
tail -n 120 /mnt/d/Projects/TypeVoice/tmp/typevoice-logs/tauri-latest-run.txt
```

## 1. Mental Model

There are two separate environments:

- WSL filesystem and Linux tooling for editing.
- Windows runtime and Windows tooling for the MVP desktop app.

Tauri dev must run with the Windows toolchain to launch a real Windows `typevoice-desktop.exe`.

## 2. Repo Layout

Recommended Windows-side working copy:

- Windows repo: `D:\Projects\TypeVoice`

Avoid running Windows builds from a UNC path such as `\\wsl.localhost\...`.

## 3. Windows Gate

Run the full local gate from Windows PowerShell:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask gate windows
```

Run only the Windows compile gate:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask gate windows-compile
```

Prepare the FFmpeg toolchain explicitly:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask toolchain ffmpeg --platform all
```

## 4. Direct Tauri Dev Command

The direct command remains available for focused debugging:

```powershell
Set-Location D:\Projects\TypeVoice\apps\desktop
$env:RUST_BACKTRACE = "1"
$env:RUST_LOG = "debug"
npm run tauri dev
```

## 5. Check Whether The App Is Running

In Windows PowerShell:

```powershell
Get-Process -Name typevoice-desktop -ErrorAction SilentlyContinue |
  Select-Object -First 1 |
  Format-List -Property Id,StartTime,Responding
```

From WSL:

```bash
/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Get-Process -Name typevoice-desktop -ErrorAction SilentlyContinue | Select-Object -First 1 | Format-List -Property Id,StartTime,Responding"
```

## 6. Stop All Dev Processes

In Windows PowerShell:

```powershell
taskkill /IM typevoice-desktop.exe /F
taskkill /IM node.exe /F
```

## 7. Common Pitfalls And Fixes

### UNC paths

Run Windows gate commands from `D:\Projects\TypeVoice`.

### WSL behavior differs from Windows behavior

Treat Windows as the runtime source of truth. Use `cargo xtask run latest` or `cargo xtask gate windows` from the Windows repo root.

### Dev console disappears after crash

Use `cargo xtask run latest`; it writes logs to `tmp/typevoice-logs/tauri-latest-run.txt`.
