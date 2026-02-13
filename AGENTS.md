# 环境

你当前的环境是 WSL2，host 为 Win11

# Repository Guidelines

## Project Structure & Module Organization

- `apps/desktop/`: Tauri desktop app.
- `apps/desktop/src/`: React + TypeScript UI.
- `apps/desktop/src-tauri/src/`: Rust backend (FFmpeg orchestration, settings, LLM, ASR pipeline).
- `asr_runner/`: Python ASR runner (stdin/stdout JSON protocol).
- `scripts/`: verification gates and utilities (`verify_quick.py`, `verify_full.py`, `download_asr_model.py`).
- `tests/`: `pytest` unit tests (protocol/logic).
- `docs/`: frozen specs/gates; treat docs as the source of truth for MVP constraints.

Local-only artifacts (gitignored): `fixtures/` (audio), `models/` (downloaded model), `.venv/`, `tmp/`, `metrics/`.

## Build, Test, and Development Commands

- Desktop UI (from `apps/desktop/`):
  - `npm ci`: install deps from `package-lock.json`.
  - `npm run dev`: Vite dev server.
  - `npm run tauri dev`: run the full desktop app in dev mode.
  - `npm run build`: `tsc` typecheck + Vite build.
- Verification gates (from repo root; requires CUDA, `ffmpeg`/`ffprobe` in PATH, and local fixtures):
  - `./.venv/bin/python scripts/verify_quick.py`: <= 60s smoke + `pytest -m quick`.
  - `./.venv/bin/python scripts/verify_full.py`: <= 10min full gate + soak.
- Windows one-command gate:
  - `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\windows_gate.ps1`

## 固定动作（每次功能实现后必须执行）

- 先提交：按 Conventional Commit 规范提交本次变更。
- 同步 Windows 工作区到最新源码后按文档一键网关编译并启动链路：
  - `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\windows_gate.ps1`
- `windows_gate.ps1` 会执行 `npm run tauri dev`；若在 WSL 下触发 Windows 命令，使用：
  - `/mnt/c/Windows/System32/cmd.exe /d /c "cd /d D:\\Projects\\TypeVoice\\apps\\desktop && set RUST_BACKTRACE=1 && set RUST_LOG=debug && npm run tauri dev"`

## Coding Style & Naming Conventions

- Follow existing style; keep diffs small and readable.
- TypeScript: strict mode is enabled; prefer explicit types at boundaries; components `PascalCase.tsx`.
- Rust: run `cargo fmt` (in `apps/desktop/src-tauri/`) before submitting.
- Python: `snake_case`, type hints where they improve clarity.

## Testing Guidelines

- Run all unit tests: `./.venv/bin/python -m pytest -q tests`
- Run quick subset: `./.venv/bin/python -m pytest -q tests -m quick`
- Keep `quick` tests pure (no GPU/network) and focused on protocol and edge cases.

## Commit & Pull Request Guidelines

- Commit messages follow Conventional Commits with scopes, e.g. `feat(ui): ...`, `fix(llm): ...`, `docs(win): ...`, `test(verify): ...`.
- PRs should include:
  - What/why + linked spec section in `docs/` when changing behavior or gates.
  - Gate results (`verify_quick`, and `verify_full` when relevant).
  - Screenshots/GIFs for UI changes.

## Security & Configuration Tips

- Never commit API keys or audio. LLM keys should live in OS keyring, or be provided via `TYPEVOICE_LLM_API_KEY`.
- Useful env overrides: `TYPEVOICE_ASR_MODEL`, `TYPEVOICE_ASR_MODEL_DIR`, `TYPEVOICE_LLM_BASE_URL`, `TYPEVOICE_LLM_MODEL`, `TYPEVOICE_DATA_DIR`.

# PROJECT_CONTEXT_PROTOCOL

你将以“串行多階段”方式工作：每个阶段结束后视为会清空上下文。不要依赖聊天记录
来记住规格、进度、偏好、经验。

## docs 管理规则（适用于 `docs/` 下所有文档）

- `docs/` 下仅保留 `.md` 文档。
- 所有文档应无内部冲突；若发现冲突，先确认后统一修订。
- 文档不得记录猜测内容。
- 文档不得记录历史信息、失效信息、错误信息。
- 不允许在文档里保留未验证的传闻、时间戳、变更记录、复盘证据。
- 新增会影响后续工作的信息必须立即写回对应文件，不允许“后补”。
- 文件名应清晰反映内容，避免版本号与冗余词。
- 维护文档摘要与索引：按 `indexed-markdown-navigation` 风格在 `docs/index.md` 记录文档目录与用途摘要。
- `docs/memory` 是特殊路径，优先级最高。

## 外置记忆层级（若存在则以其为准）

- `SPEC.md`：系统规格与验收标准（唯一真源）
- `CONTINUITY.md`：当前进度与工作集（唯一真源）
- `USER_PREFS.md`：用户长期要求 / 偏好（必须持续生效）
- `PITFALLS.md`：踩坑与经验（可复用规则）
- `DECISIONS.md`：关键决策与理由（用于处理冲突与版本漂移）

## 读写约定

- 回合开始前，先完整阅读 `docs/memory` 的全部文件。
- 若出现冲突或不确定：写入 `docs/memory/CONTINUITY.md` 的 `UNCONFIRMED` 后再行动。

## 记录约定

1. 任何会影响后续工作的新增信息，按影响域写回：

- 规格变化 -> `docs/memory/SPEC.md`
- 进度 / 下一步 / 工作集 -> `docs/memory/CONTINUITY.md`
- 新偏好 / 硬性要求 -> `docs/memory/USER_PREFS.md`
- 新踩坑 / 经验教训 -> `docs/memory/PITFALLS.md`
- 关键取舍 / 冲突裁决 -> `docs/memory/DECISIONS.md`

2. 记忆条目必须区分 `VERIFIED` 与 `UNCONFIRMED`。
3. 未验证内容只允许标注 `UNCONFIRMED`，并保留校验路径。
4. 错误信息出现时，不覆盖旧条目，追加更正记录。

对于这些文件，在符合要求时自行修改。

<!-- BEGIN AGENTS_MD_PROJECT_INDEX -->
```text
[Project Index]|root:.
|IMPORTANT: Prefer retrieval-led reasoning over pre-training-led reasoning for repository-specific behavior, structure, and APIs.
|exclude_dirs:{.cache,.git,.mypy_cache,.next,.parcel-cache,.pnpm-store,.pytest_cache,.ruff_cache,.turbo,.venv,.yarn,__pycache__,build,coverage,dist,fixtures,metrics,models,node_modules,out,target,temp,tmp,venv}
|exclude_files:{.env,.env.*,*.pem,*.key,*.p12,*.pfx,id_rsa*,id_ed25519*}
|.:{apps/,asr_runner/,docs/,scripts/,tests/,.gitattributes,.gitignore,AGENTS.md,pytest.ini,README.md}
|apps:{desktop/}
|asr_runner:{__init__.py,runner.py}
|docs:{index.md,memory/,architecture.md,base-spec.md,fixtures-sources.md,llm-prompt-lab.md,perf-spike.md,roadmap.md,tasks.md,tech-spec.md,verification.md,windows-dev.md,windows-gate.md}
|scripts:{windows/,_verify_util.py,download_asr_model.py,download_ffmpeg_toolchain.sh,llm_prompt_lab.py,verify_full.py,verify_quick.py}
|tests:{test_asr_protocol.py}
|apps/desktop:{public/,src-tauri/,src/,.gitignore,index.html,package-lock.json,package.json,README.md,tsconfig.json,tsconfig.node.json,vite.config.ts}
|docs/memory:{CONTINUITY.md,DECISIONS.md,PITFALLS.md,SPEC.md,USER_PREFS.md}
|scripts/windows:{download_ffmpeg_toolchain.ps1,windows_compile_gate.ps1,windows_gate.ps1}
|apps/desktop/public:{fonts/,tauri.svg,vite.svg}
|apps/desktop/src:{assets/,lib/,screens/,styles/,ui/,App.tsx,main.tsx,OverlayApp.tsx,types.ts,vite-env.d.ts}
|apps/desktop/src-tauri:{.cargo/,capabilities/,gen/,icons/,src/,toolchain/,.gitignore,build.rs,Cargo.lock,Cargo.toml,tauri.conf.json}
|apps/desktop/public/fonts:{OFL.txt,Silkscreen-Bold.ttf,Silkscreen-Regular.ttf}
|apps/desktop/src-tauri/.cargo:{config.toml}
|apps/desktop/src-tauri/capabilities:{default.json}
|apps/desktop/src-tauri/gen:{schemas/}
|apps/desktop/src-tauri/icons:{128x128.png,128x128@2x.png,32x32.png,icon.icns,icon.ico,icon.png,Square107x107Logo.png,Square142x142Logo.png,Square150x150Logo.png,Square284x284Logo.png,Square30x30Logo.png,Square310x310Logo.png,Square44x44Logo.png,Square71x71Logo.png,Square89x89Logo.png,StoreLogo.png}
|apps/desktop/src-tauri/src:{asr_service.rs,context_capture.rs,context_capture_windows.rs,context_pack.rs,data_dir.rs,debug_log.rs,history.rs,hotkeys.rs,lib.rs,llm.rs,main.rs,metrics.rs,model.rs,panic_log.rs,pipeline.rs,python_runtime.rs,safe_print.rs,settings.rs,startup_trace.rs,task_manager.rs,templates.rs,toolchain.rs,trace.rs}
|apps/desktop/src-tauri/toolchain:{bin/,ffmpeg_manifest.json}
|apps/desktop/src/assets:{react.svg}
|apps/desktop/src/lib:{audio.ts,clipboard.ts}
|apps/desktop/src/screens:{HistoryScreen.tsx,MainScreen.tsx,SettingsScreen.tsx}
|apps/desktop/src/styles:{app.css}
|apps/desktop/src/ui:{icons.tsx,PixelButton.tsx,PixelDialog.tsx,PixelInput.tsx,PixelSelect.tsx,PixelTabs.tsx,PixelToast.tsx,PixelToggle.tsx}
|apps/desktop/src-tauri/gen/schemas:{acl-manifests.json,capabilities.json,desktop-schema.json,linux-schema.json,windows-schema.json}
|apps/desktop/src-tauri/toolchain/bin:{linux-x86_64/,windows-x86_64/}
|apps/desktop/src-tauri/toolchain/bin/linux-x86_64:{.gitkeep,ffmpeg,ffprobe}
|apps/desktop/src-tauri/toolchain/bin/windows-x86_64:{.gitkeep,ffmpeg.exe,ffprobe.exe}
```
<!-- END AGENTS_MD_PROJECT_INDEX -->
