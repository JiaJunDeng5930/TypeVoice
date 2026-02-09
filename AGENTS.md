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
- PITFALLS.md：踩坑与经验（必须可复用）
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
