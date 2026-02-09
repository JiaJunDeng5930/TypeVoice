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
- 2026-02-09 全工程 review 已完成并记录（本回合已验证：`pytest -m quick`、`cargo test`、`npm run build` 均通过）。

## Review Findings（2026-02-09）

VERIFIED（来自代码审阅与本机可复核命令）

- [Critical] UI 侧没有可达的“取消任务”入口，不符合冻结规格“任何阶段可取消（<=300ms）”：
- 证据：`apps/desktop/src/screens/MainScreen.tsx` 未调用 `invoke("cancel_task")`，且转录中按钮禁用 `disabled={ui === "transcribing"}`。
- 后端虽提供命令：`apps/desktop/src-tauri/src/lib.rs` 有 `cancel_task` command，`apps/desktop/src-tauri/src/task_manager.rs` 有 `TaskManager.cancel()`。
- [Critical] `TaskManager` 顶层错误会被吞掉，可能导致 UI 卡死在 TRANSCRIBING（无 `task_event`/`task_done`）：
- 证据：`apps/desktop/src-tauri/src/task_manager.rs` 中 `run_pipeline(...).await` 的结果被 `let _ = ...` 丢弃，且部分路径使用 `?` 可能直接返回错误而未 emit 事件（例如 Rewrite 阶段模板读取）。
- [High] FFmpeg 失败诊断与错误码不对齐技术规格：
- 证据：`apps/desktop/src-tauri/src/pipeline.rs` 的 ffmpeg 调用丢弃 stderr（`Stdio::null()`），`apps/desktop/src-tauri/src/task_manager.rs` 预处理失败上报 `E_PREPROCESS_FAILED`，但 `docs/tech-spec-v0.1.md` 建议 `E_FFMPEG_NOT_FOUND`/`E_FFMPEG_FAILED` 且需要 stderr 摘要用于 UI 诊断。
- [High] FFmpeg 内置并可定位尚未落地，当前依赖 PATH：
- 证据：`apps/desktop/src-tauri/src/pipeline.rs` 通过 `Command::new("ffmpeg")`；技术规格要求 Windows 安装包内置 `ffmpeg.exe` 并可在运行时定位（`docs/tech-spec-v0.1.md`）。
- [High] 模型校验不足，不满足 hash/manifest/版本记录等冻结要求：
- 证据：`apps/desktop/src-tauri/src/model.rs` 仅校验 `model_dir` 与 `config.json` 存在；技术规格 `docs/tech-spec-v0.1.md` 要求至少 hash/manifest + 版本标识记录。
- [High] 指标落盘吞 I/O 错误，观测数据可能静默丢失：
- 证据：`apps/desktop/src-tauri/src/metrics.rs` 使用 `write_all(...).ok()`。
- [High] LLM 失败信息可能过长并原样进入 metrics/message（缺少截断/脱敏），与“简要错误摘要”目标不匹配：
- 证据：`apps/desktop/src-tauri/src/llm.rs` 在非 2xx 时返回 `body` 全文，`apps/desktop/src-tauri/src/task_manager.rs` 会将错误 `to_string()` 作为事件 message，并写入 metrics。
- [Medium] settings 解析失败会被当作默认值（行为“静默回退”），可能掩盖配置损坏：
- 证据：`apps/desktop/src-tauri/src/lib.rs` 的 `update_settings` 使用 `unwrap_or_default()`。
- [Medium] ASR Runner 的 `model_version` 当前恒为 `None`，不满足技术规格中“输出 model_version”的建议字段：
- 证据：`asr_runner/runner.py` 固定填 `model_version=None`，而 `scripts/download_asr_model.py` 会写入 `REVISION.txt` 可用于填充。
- [Medium] 测试覆盖偏薄：主要覆盖 runner protocol-only，未覆盖 TaskManager 事件序列与取消等硬约束路径：
- 证据：`tests/test_asr_protocol.py`。

UNCONFIRMED（需要在 Windows 打包/运行环境或 full gate 进一步验证）

- ffmpeg 内置定位方案（Tauri bundle resources 路径、命名与许可）如何实现与验收。
- `verify_full` 在当前机器是否可通过（GPU/fixtures/模型安装齐备时）。

## Fixes Applied（2026-02-09）

VERIFIED（已合并到 git main，且本机通过：`cargo test`、`pytest`、`npm run build`）

- P0：UI 已接入取消任务（转录中点击主按钮触发取消，取消中禁用重复操作）。
- P0：TaskManager 顶层 Err 现在会发出 fail-safe `task_event`，避免 UI 卡死；Rewrite 模板读取失败不再导致整个任务无事件退出。
- P1：FFmpeg 预处理失败现在携带 stderr 摘要；错误码映射为 `E_FFMPEG_NOT_FOUND`/`E_FFMPEG_FAILED`（否则 `E_PREPROCESS_FAILED`），并支持 env 覆盖 `TYPEVOICE_FFMPEG`/`TYPEVOICE_FFPROBE`。
- P1：模型下载脚本写入 `REVISION.txt` 与 `manifest.json`；Rust 侧 `asr_model_status` 会校验 manifest 并返回 `model_version`；ASR Runner 会在 metrics 中填充 `model_version`（本地目录模型）。
- P2：metrics 落盘不再吞 I/O 错误（改为传播，并在 emit 处记录 stderr）；LLM 失败 body 截断；settings 解析失败会自动备份 `settings.json` 并恢复默认值而非静默吞掉。

Commit References（VERIFIED）

- `d3de362`：`fix(core): enable cancel and harden task pipeline`
- `518fe18`：`feat(model): add manifest and version metadata`

UNCONFIRMED

- `verify_quick` / `verify_full` 是否在本机当前环境可直接通过（本回合未执行）。
- Windows 目标机上的原生 Gate（`scripts/windows/windows_gate.ps1`）是否已经跑通并留档。

## Next

UNCONFIRMED

- 明确 `docs/memory/SPEC.md` 的权威性边界：
- 是否仅作为索引与最小摘要（推荐，避免与 `docs/*-spec*.md` 漂移），还是要逐步迁移为唯一真源。
- 运行并记录一次验证结果：repo root 执行 `./.venv/bin/python scripts/verify_quick.py` 与 `./.venv/bin/python scripts/verify_full.py`。
- 在 Windows 原生环境跑 M12 Gate，并将结果写入本外置记忆（或另建 `docs/memory/DECISIONS.md` / `PITFALLS.md` 视情况追加）。
- 修复优先级（建议按严重度）：
- P0：补齐 UI 取消入口（对齐“任何阶段可取消”）。
- P0：保证任意失败路径都能 emit `task_event(status=failed|cancelled)`，避免 UI 卡死。
- P1：对齐 FFmpeg 失败诊断（stderr 摘要）与错误码映射。
- P1：补齐 model 校验与 version/manifest 记录。
- P2：修复 metrics 落盘吞错、LLM 错误消息截断/脱敏、settings load 错误提示。

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
