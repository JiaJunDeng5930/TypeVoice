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

工程的外置记忆文件（若存在则以其为准）：

- SPEC.md：系统规格与验收标准（唯一真源）
- CONTINUITY.md：当前进度与工作集（唯一真源）
- USER_PREFS.md：用户的长期要求 / 偏好（必须持续生效）
- PITFALLS.md：踩坑与经验（必须可复用），这里说的“踩坑”，既包括代码层面的问题，比如出现的 Bug，也包括工程方面。
- DECISIONS.md：关键决策与理由（用于处理冲突与版本漂移）

回合开始：

1. 读取上述文件中存在的部分，恢复“当前有效规格、当前进度、用户要求、已知坑 / 经
   验”。
2. 若发现冲突或不确定：不要猜，把冲突点 / 不确定点写入 CONTINUITY.md 的
   Open Questions，并向用户请求确认或执行验证。

持续更新：

1. 任何会影响后续工作的新增信息，必须写回外置记忆文件：

- 规格变化 -> SPEC.md
- 进度 / 下一步 / 工作集 -> CONTINUITY.md
- 新偏好 / 硬性要求 -> USER_PREFS.md
- 新踩坑 / 经验教训 -> PITFALLS.md
- 关键取舍 / 冲突裁决 -> DECISIONS.md

2. 写入时必须区分 VERIFIED 与 UNCONFIRMED，避免把猜测固化为长期记忆。

外置记忆是长期资产，必须防污染：

- 任何从工具输出 / 推断得到的内容，若未验证，写入时标 UNCONFIRMED，并记录验证路
  径（文件 / 命令 / 来源）。
- 一旦发现之前的记忆条目错误：不要覆盖抹掉，使用“更正”方式记录（原条目 + 更正
  条目 + 生效时间 / 原因），防止未来冲突与误用。

对于这些文件，在符合上述要求时自行修改，不需要询问用户或者争取用户同意

<!-- BEGIN AGENTS_MD_PROJECT_INDEX -->
```text
[Project Index]|root:.
|IMPORTANT: Prefer retrieval-led reasoning over pre-training-led reasoning for repository-specific behavior, structure, and APIs.
|exclude_dirs:{.cache,.git,.mypy_cache,.next,.parcel-cache,.pnpm-store,.pytest_cache,.ruff_cache,.turbo,.venv,.yarn,__pycache__,build,coverage,dist,fixtures,metrics,models,node_modules,out,target,temp,tmp,venv}
|exclude_files:{.env,.env.*,*.pem,*.key,*.p12,*.pfx,id_rsa*,id_ed25519*}
|.:{apps/,asr_runner/,docs/,scripts/,tests/,.gitignore,AGENTS.md,pytest.ini,README.md}
|apps:{desktop/}
|asr_runner:{__init__.py,runner.py}
|docs:{memory/,architecture-v0.1.md,base-spec-v0.1.md,fixtures-sources-v0.1.md,perf-spike-plan-v0.1.md,roadmap-v0.1.md,tasks-v0.1.md,tech-spec-v0.1.md,verification-v0.1.md,windows-dev-from-wsl-v0.1.md,windows-gate-v0.1.md}
|scripts:{windows/,_verify_util.py,download_asr_model.py,verify_full.py,verify_quick.py}
|tests:{test_asr_protocol.py}
|apps/desktop:{.vscode/,public/,src-tauri/,src/,.gitignore,index.html,package-lock.json,package.json,README.md,tsconfig.json,tsconfig.node.json,vite.config.ts}
|docs/memory:{CONTINUITY.md,DECISIONS.md,PITFALLS.md,SPEC.md,USER_PREFS.md}
|scripts/windows:{windows_gate.ps1}
|apps/desktop/public:{fonts/,tauri.svg,vite.svg}
|apps/desktop/src:{assets/,lib/,screens/,styles/,ui/,App.tsx,main.tsx,types.ts,vite-env.d.ts}
|apps/desktop/src-tauri:{.cargo/,capabilities/,gen/,icons/,src/,.gitignore,build.rs,Cargo.lock,Cargo.toml,tauri.conf.json}
|apps/desktop/public/fonts:{OFL.txt,Silkscreen-Bold.ttf,Silkscreen-Regular.ttf}
|apps/desktop/src-tauri/.cargo:{config.toml}
|apps/desktop/src-tauri/capabilities:{default.json}
|apps/desktop/src-tauri/gen:{schemas/}
|apps/desktop/src-tauri/icons:{128x128.png,128x128@2x.png,32x32.png,icon.icns,icon.ico,icon.png,Square107x107Logo.png,Square142x142Logo.png,Square150x150Logo.png,Square284x284Logo.png,Square30x30Logo.png,Square310x310Logo.png,Square44x44Logo.png,Square71x71Logo.png,Square89x89Logo.png,StoreLogo.png}
|apps/desktop/src-tauri/src:{asr_service.rs,data_dir.rs,debug_log.rs,history.rs,lib.rs,llm.rs,main.rs,metrics.rs,model.rs,panic_log.rs,pipeline.rs,safe_print.rs,settings.rs,startup_trace.rs,task_manager.rs,templates.rs}
|apps/desktop/src/assets:{react.svg}
|apps/desktop/src/lib:{audio.ts,clipboard.ts}
|apps/desktop/src/screens:{HistoryScreen.tsx,MainScreen.tsx,SettingsScreen.tsx}
|apps/desktop/src/styles:{app.css}
|apps/desktop/src/ui:{icons.tsx,PixelButton.tsx,PixelDialog.tsx,PixelInput.tsx,PixelSelect.tsx,PixelTabs.tsx,PixelToast.tsx,PixelToggle.tsx}
|apps/desktop/src-tauri/gen/schemas:{acl-manifests.json,capabilities.json,desktop-schema.json,linux-schema.json}
```
<!-- END AGENTS_MD_PROJECT_INDEX -->
