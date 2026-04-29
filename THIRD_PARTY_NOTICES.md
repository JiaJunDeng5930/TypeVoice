# Third-Party Notices

This project redistributes and/or integrates third-party components.

## FFmpeg

- Component: FFmpeg binaries (`ffmpeg`, `ffprobe`)
- Version: `7.0.2`
- Bundled paths:
  - `apps/desktop/src-tauri/toolchain/bin/windows-x86_64/`
  - `apps/desktop/src-tauri/toolchain/bin/linux-x86_64/`
- Binary sources:
  - Windows: `https://github.com/GyanD/codexffmpeg/releases/download/7.0.2/ffmpeg-7.0.2-essentials_build.zip`
  - Linux: `https://johnvansickle.com/ffmpeg/releases/ffmpeg-7.0.2-amd64-static.tar.xz`
- License notice:
  - Current bundled builds are GPL route builds (`--enable-gpl` with `libx264/libx265`), therefore distributed under GPLv3-or-later terms for those binaries.
- Supply-chain verification:
  - `cargo xtask toolchain ffmpeg` verifies FFmpeg upstream source archive signature against
    `https://ffmpeg.org/ffmpeg-devel.asc` and pinned fingerprint
    `FCF986EA15E6E293A5644F10B4322F04D67658D8` before consuming prebuilt binaries.
  - Verification metadata is pinned in `apps/desktop/src-tauri/toolchain/ffmpeg_manifest.json`.
- Corresponding source references:
  - `https://github.com/GyanD/codexffmpeg/tree/7.0.2`
  - `https://johnvansickle.com/ffmpeg/release-source/`
  - Upstream FFmpeg legal page: `https://ffmpeg.org/legal.html`

## npm/cargo dependencies

This repository also uses third-party dependencies from:
- npm (`apps/desktop/package-lock.json`)
- Cargo (`apps/desktop/src-tauri/Cargo.lock`)
- Cargo (`Cargo.lock`)

Their licenses are governed by each upstream package.
