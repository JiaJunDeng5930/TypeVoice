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
