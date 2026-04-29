# Windows Gate

This repo is primarily developed in WSL, but the target runtime for the MVP is Windows desktop.

For the detailed WSL and Windows workflow, see:

- `docs/windows-dev.md`

## One-Command Gate

Run in a Windows PowerShell terminal from the Windows repo root:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask gate windows
```

This command:

- checks `node`, `npm`, `cargo`, and `gpg`;
- runs the Windows Rust compile gate;
- downloads and verifies the controlled FFmpeg toolchain;
- runs `npm ci`;
- runs `npm run build`;
- runs Rust tests;
- starts `npm run tauri dev`.

## Latest-Code Launch

For day-to-day launch and debug:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask run latest
```

This command stops stale TypeVoice desktop processes tied to the repo, installs desktop npm dependencies, builds the frontend, starts Tauri dev, waits for the Windows app process, and prints the latest log tail.

## Fast Gate: Windows Compile Check

If you only need to check whether the Windows Rust backend compiles:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask gate windows-compile
```

## FFmpeg Toolchain

To prepare both bundled toolchains:

```powershell
Set-Location D:\Projects\TypeVoice
cargo xtask toolchain ffmpeg --platform all
```

The toolchain command verifies the upstream FFmpeg source signature with `gpg`, checks pinned sha256 values, and installs binaries under `apps/desktop/src-tauri/toolchain/bin/...`.

## Command Discipline

When this document is used as the execution source, follow commands exactly as written.

- No command rewrite.
- No extra wrappers.
- No extra pre/post steps.
- If a command fails, return the original error first and stop.
- Only use remediation commands explicitly documented in this file or `docs/windows-dev.md`.

## Expected Failures

The gate intentionally fails fast if any of these are missing:

- `node`, `npm`, `cargo` are unavailable.
- `gpg` is unavailable.
- bundled FFmpeg toolchain download/checksum/signature validation fails.
