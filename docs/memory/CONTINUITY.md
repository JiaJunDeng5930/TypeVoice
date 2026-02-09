# CONTINUITY（外置记忆）

说明：

- 本文件记录“当前进度与工作集”，用于跨回合恢复上下文。
- 内容必须区分 `VERIFIED`（能用代码/文档/命令复核）与 `UNCONFIRMED`（推断/未验证）。

## Now

VERIFIED

- 外置记忆文件尚未存在，现已在 `docs/memory/` 初始化最小骨架：`docs/memory/SPEC.md`、`docs/memory/CONTINUITY.md`、`docs/memory/USER_PREFS.md`。
- 外置记忆初始化已提交到 git（commit `97564c7`：`docs(memory): init external memory scaffolding`）。
- 冻结规格与 Gate 的真源在 `docs/*.md`，`README.md` 明确“只有写入文档的内容才视为可靠约束与验收依据”。
- 工程结构与主要入口：Desktop `apps/desktop/`（Vite + React + TS），Tauri Rust 入口 `apps/desktop/src-tauri/src/main.rs`、`apps/desktop/src-tauri/src/lib.rs`；ASR Runner `asr_runner/runner.py`（stdin/stdout JSON，支持 `--protocol-only`）；验证门禁 `scripts/verify_quick.py`、`scripts/verify_full.py`（结构化指标落盘 `metrics/verify.jsonl`）；后端任务事件由 Rust `TaskManager` emit `task_event` / `task_done` 并写入数据目录的 `metrics.jsonl`（默认 `tmp/typevoice-data/metrics.jsonl`，见 `apps/desktop/src-tauri/src/task_manager.rs`、`apps/desktop/src-tauri/src/metrics.rs`、`apps/desktop/src/screens/MainScreen.tsx`）。
- 默认数据目录：`TYPEVOICE_DATA_DIR` 可覆盖；否则默认 `tmp/typevoice-data`（开发默认）。来源：`apps/desktop/src-tauri/src/data_dir.rs`。

UNCONFIRMED

- `verify_quick` / `verify_full` 是否在本机当前环境可直接通过（本回合未执行）。
- Windows 目标机上的原生 Gate（`scripts/windows/windows_gate.ps1`）是否已经跑通并留档。

## Next

UNCONFIRMED

- 明确 `docs/memory/SPEC.md` 的权威性边界：
- 是否仅作为索引与最小摘要（推荐，避免与 `docs/*-spec*.md` 漂移），还是要逐步迁移为唯一真源。
- 运行并记录一次验证结果：repo root 执行 `./.venv/bin/python scripts/verify_quick.py` 与 `./.venv/bin/python scripts/verify_full.py`。
- 在 Windows 原生环境跑 M12 Gate，并将结果写入本外置记忆（或另建 `docs/memory/DECISIONS.md` / `PITFALLS.md` 视情况追加）。

## Open Questions

UNCONFIRMED

- 分发形态对齐：技术规格要求“内置 FFmpeg 随包分发”，但当前开发实现里 `apps/desktop/src-tauri/src/pipeline.rs` 通过 `Command::new(\"ffmpeg\")` 依赖 PATH（打包时如何定位内置 ffmpeg？是否已有实现但未在此处体现？）。
- 模型校验深度：当前 `apps/desktop/src-tauri/src/model.rs` 的校验是“目录存在 + config.json 存在”的最小集合；是否要对齐 `docs/tech-spec-v0.1.md` 所要求的 hash/manifest 校验与版本记录。
- ASR Runner 的分发策略：当前依赖 repo-local `.venv` + `python -m asr_runner.runner`；Windows 发布时是否采用嵌入式 Python 或其他方式（技术规格列为待确认）。

## Working Set

VERIFIED

- 冻结规格入口：
- `docs/base-spec-v0.1.md`；`docs/tech-spec-v0.1.md`；`docs/verification-v0.1.md`；`docs/roadmap-v0.1.md`；`docs/tasks-v0.1.md`。
- 核心实现文件（优先阅读）：
- `apps/desktop/src-tauri/src/task_manager.rs`；`apps/desktop/src-tauri/src/pipeline.rs`；`apps/desktop/src-tauri/src/llm.rs`；`apps/desktop/src-tauri/src/settings.rs`；`asr_runner/runner.py`。
- 验证与测试：
- `scripts/verify_quick.py`；`scripts/verify_full.py`；`tests/test_asr_protocol.py`。
