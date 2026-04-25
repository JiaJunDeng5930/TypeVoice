# Contributing

Thanks for your interest in TypeVoice.

## Before You Start

- Read [README.md](./README.md) for setup and usage.
- Read [docs/index.md](./docs/index.md) for specification and architecture docs.

## Development Setup

1. Install dependencies:
   - `cd apps/desktop && npm ci`
   - `cd /path/to/TypeVoice && cargo run --locked --manifest-path tools/typevoice-tools/Cargo.toml -- fixtures download`
2. Start desktop app:
   - `cd apps/desktop && npm run tauri dev`

## Testing

Run at repo root:

- `cargo test --locked --manifest-path tools/typevoice-tools/Cargo.toml`
- `cargo run --locked --manifest-path tools/typevoice-tools/Cargo.toml -- verify quick`
- `cargo run --locked --manifest-path tools/typevoice-tools/Cargo.toml -- verify full`

## Pull Request Guidelines

- Keep changes focused and atomic.
- Follow Conventional Commits.
- Update docs when behavior or workflow changes.
- Include verification results in PR description.
