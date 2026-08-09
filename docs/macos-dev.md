# macOS Development

TypeVoice supports source builds on Apple Silicon Macs.

## Prepare the controlled FFmpeg toolchain

Run from the repository root:

```bash
cargo xtask toolchain ffmpeg --platform macos-aarch64
```

The command verifies the pinned FFmpeg release and installs `ffmpeg` and `ffprobe` under `apps/desktop/src-tauri/toolchain/bin/macos-aarch64/`.

## Launch the desktop app

```bash
cd apps/desktop
npm ci
npm run tauri dev
```

macOS asks for Microphone access when audio capture starts. Global hotkeys and automatic paste also require Accessibility access. Grant TypeVoice access in **System Settings → Privacy & Security → Accessibility**, then restart the app.

## Build an application bundle

```bash
cd apps/desktop
npm run tauri build -- --bundles app
```

The bundle is written below the workspace `target/release/bundle/macos/` directory.
