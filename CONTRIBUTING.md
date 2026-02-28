# Contributing

Thanks for your interest in TypeVoice.

## Before You Start

- Read [README.md](./README.md) for setup and usage.
- Read [docs/index.md](./docs/index.md) for specification and architecture docs.

## Development Setup

1. Install dependencies:
   - `cd apps/desktop && npm ci`
   - `cd /path/to/TypeVoice && ./.venv/bin/python -m pip install -r requirements.txt`
2. Start desktop app:
   - `cd apps/desktop && npm run tauri dev`

## Testing

Run at repo root:

- `./.venv/bin/python -m pytest -q tests`
- `./.venv/bin/python scripts/verify_quick.py`
- `./.venv/bin/python scripts/verify_full.py` (requires local fixtures/model and GPU)

## Pull Request Guidelines

- Keep changes focused and atomic.
- Follow Conventional Commits.
- Update docs when behavior or workflow changes.
- Include verification results in PR description.
