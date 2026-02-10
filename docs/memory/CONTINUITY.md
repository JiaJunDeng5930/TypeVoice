# CONTINUITY（外置记忆）

说明：

- 本文件记录“当前进度与工作集”，用于跨回合恢复上下文。
- 内容必须区分 `VERIFIED`（能用代码/文档/命令复核）与 `UNCONFIRMED`（推断/未验证）。
- 冻结规格的真源在 `docs/*.md`；本文件只做“可执行的接手摘要”，避免复述导致漂移（VERIFIED：`docs/memory/SPEC.md`）。

## 当前有效目标（与 SPEC 对齐）

VERIFIED（截至 2026-02-10）

- 目标：Windows 桌面端“录完再出稿”语音打字工具（录音结束 -> 本地 ASR -> 可选 LLM Rewrite -> 一键复制），优先可用性/稳定性/速度/可取消/可观测。真源：`docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`、`docs/verification-v0.1.md`。
- 本阶段目标（性能与可观测性）：
- 性能优化：基于落盘指标/日志梳理各阶段耗时，定位主要瓶颈并给出改进方案。
- 可观测性增强：在 debug 开关开启时，必须能在日志中看到：
  - 音频切分信息（分几段、每段起止/时长）；
  - 每段 ASR 转录文本；
  - 每次 LLM 请求的完整输入（system/user、拼接上下文等）；
  - LLM 原始返回内容。
- 常驻能力：ASR runner 在程序启动后常驻后台（减少首条转录延迟），并可在设置变更时重启。

## 当前状态（Done / Now / Next）

### Done

VERIFIED（已合并到 main，且可在本机复核）

- Windows 快速质量闸门补强：新增 Windows 编译闸门脚本 `scripts/windows/windows_compile_gate.ps1`，用于快速捕获 Windows-only 编译错误（例如 Tauri state 的 `Send/Sync` 约束问题）；并已接入 `scripts/windows/windows_gate.ps1`，会在重步骤（torch/model/npm）前 fail-fast。
- Debug verbose 日志链路已实现：
  - 仅在 `TYPEVOICE_DEBUG_VERBOSE=1` 时才落“ASR 分段信息 / LLM 请求与返回”的完整 payload：
    - payload 落盘：`TYPEVOICE_DATA_DIR/debug/<task_id>/asr_segments.json|llm_request.json|llm_response.txt`
    - 同时在 `TYPEVOICE_DATA_DIR/metrics.jsonl` 追加 `debug_*` 事件，引用 `payload_path`（见 `apps/desktop/src-tauri/src/debug_log.rs`、`apps/desktop/src-tauri/src/asr_service.rs`、`apps/desktop/src-tauri/src/llm.rs`）。
- ASR runner 常驻已实现：
  - 启动时后台 warmup，常驻复用；设置里 ASR 模型变更会 best-effort 重启（见 `apps/desktop/src-tauri/src/task_manager.rs`、`apps/desktop/src-tauri/src/asr_service.rs`）。
  - 可通过 env 关闭常驻用于排障：`TYPEVOICE_ASR_RESIDENT=0`（见 `apps/desktop/src-tauri/src/task_manager.rs`）。
- Windows release “localhost 无法连接/闪退”排障已完成并修复两类问题：
  - release 资源加载：启用 Tauri `custom-protocol`（见 `apps/desktop/src-tauri/Cargo.toml`）。
  - 闪退根因：`APPCRASH` / `0xc00000fd (STATUS_STACK_OVERFLOW)`；通过提升 Windows MSVC 目标主线程栈保留大小到 8MB 修复：
    - 配置：`apps/desktop/src-tauri/.cargo/config.toml`（`/STACK:8388608`）
    - 复核：`objdump -x ...typevoice-desktop.exe` 中 `SizeOfStackReserve == 0x00800000`
    - 风险与经验已写入：`docs/memory/PITFALLS.md`、`docs/memory/DECISIONS.md`。
- Windows GUI 子系统诊断补强（best-effort）：
  - `safe_eprintln!`：避免依赖不可用 stderr（`apps/desktop/src-tauri/src/safe_print.rs`）
  - panic hook 与启动面包屑落盘：`apps/desktop/src-tauri/src/panic_log.rs`、`apps/desktop/src-tauri/src/startup_trace.rs`（注意：这两者用于排障，不是业务日志）。
- Windows 编译迭代提速（源码行尾 + Rust dev 链接 + sccache，可复核）：
  - 行尾规范化：新增 `.gitattributes`，对源码/常见配置与脚本强制 LF（减少 Windows checkout 的 CRLF 漂移噪声）。
  - Rust dev profile：`apps/desktop/src-tauri/Cargo.toml` 设置 `[profile.dev] debug = 1`（降低链接开销）。
  - Windows gate：`scripts/windows/windows_gate.ps1` 可选启用 `sccache`（存在则设置 `RUSTC_WRAPPER=sccache` 与 repo-local `SCCACHE_DIR`）。
  - Windows 实测（PowerShell `Measure-Command`，在 `D:\\Projects\\TypeVoice`，使用隔离的 `CARGO_TARGET_DIR`）：
  - `cargo build`（无 sccache，clean target）：约 127s
  - `cargo build`（sccache 空缓存，clean target）：约 157s（写缓存开销）
  - `cargo build`（sccache 缓存命中，删除 target 后重建）：约 68s（`sccache --show-stats` 可见 hits）
- 增量（对 `src-tauri/src/main.rs` 加一行触发重建）：无 sccache 约 2.26s；sccache 约 1.85s
- 默认内置提示词模板已更新：
  - `apps/desktop/src-tauri/src/templates.rs` 中模板 `id="clarify"`（表达澄清）的 `system_prompt` 替换为“面向转录文本的严格重写”版本，以减少跑题/细化/省略与解释性输出（commit：`a6aa04a`）。
- Rewrite 上下文（ContextPack）已接入到后端流水线（best-effort）：
  - Rewrite 请求会自动附带：最近 N 条历史文本 + 剪贴板文本 + “TypeVoice 之前的外部前台窗口”截图（Windows-only），无需 UI 勾选或预览（见 `apps/desktop/src-tauri/src/context_capture.rs`、`apps/desktop/src-tauri/src/context_pack.rs`、`apps/desktop/src-tauri/src/task_manager.rs`）。
  - LLM 请求支持多模态 `messages[].content`（text + image_url）；且 debug verbose 落盘的 `llm_request.json` 会对截图 base64 做脱敏（不落真实像素，仅保留 sha/尺寸/字节数；见 `apps/desktop/src-tauri/src/llm.rs`）。
  - UNCONFIRMED：Windows 上对“指定窗口截图（PrintWindow + PNG 编码）”的兼容性需在目标机器上验证（不同应用窗口可能失败或空白）。

### Now

VERIFIED（当前现状）

- Windows release 版本可稳定运行，数据目录为 `D:\\Projects\\TypeVoice\\tmp\\typevoice-data`（等价于默认 `tmp/typevoice-data`）。
- 最新一次任务的 `metrics.jsonl` 中只有 `task_event/task_perf`，没有 `debug_*` 事件；且 `debug/` 目录不存在：说明当次运行时未开启 `TYPEVOICE_DEBUG_VERBOSE`，因此“分片转录/LLM 请求与回复原文”没有被记录下来（这是预期行为，不是 bug）。
- LLM rewrite 的模板系统为“可落盘覆盖”：
  - 若数据目录存在 `templates.json`，应用将优先使用该文件（UI 编辑/导入导出都会写它）；
  - 只有当 `templates.json` 不存在时，才会回退使用内置默认模板（见 `apps/desktop/src-tauri/src/templates.rs` 的 `default_templates()`）。

### Next

VERIFIED（下一步推进顺序，零上下文 agent 可直接做）

0. 确认默认提示词模板更新在目标环境“实际生效”（避免误以为代码改动没用）：
- 在目标环境定位 data dir（Windows 常见为 `D:\\Projects\\TypeVoice\\tmp\\typevoice-data`）。
- 若存在 `templates.json`，则它会覆盖内置默认模板；需要通过 UI 更新模板，或用导入 JSON 的 `replace` 覆盖，或删除该文件让应用回退到内置默认模板（此操作有用户数据语义，需谨慎）。
- 推荐验证（只读）：
  - `jq -r '.[] | select(.id==\"clarify\") | .system_prompt' tmp/typevoice-data/templates.json`（若文件存在）
1. 让“完整调试日志”可重复产出：
- 在 Windows 启动进程时设置 `TYPEVOICE_DEBUG_VERBOSE=1`（必要时再加 `TYPEVOICE_DEBUG_INCLUDE_LLM=1`、`TYPEVOICE_DEBUG_INCLUDE_ASR_SEGMENTS=1`）。
- 触发一次转录任务后，从 `tmp/typevoice-data/metrics.jsonl` 找到最后一个 `task_id`，读取：
  - `tmp/typevoice-data/debug/<task_id>/asr_segments.json`
  - `tmp/typevoice-data/debug/<task_id>/llm_request.json`
  - `tmp/typevoice-data/debug/<task_id>/llm_response.txt`
2. 做性能分析（不依赖 debug verbose）：
- 从 `metrics.jsonl` 的 `task_perf` 汇总分布（preprocess/asr/rewrite），找瓶颈（当前样本里 Rewrite 往往是最大项）。
- 若要细到“音频切分/每段耗时”，需扩展 ASR runner 协议或在 Rust 侧记录更细粒度的分段耗时（当前仅落分段文本与元信息）。

## 当前工作集（关键文件 / 命令 / 约束）

### 关键文件

VERIFIED

- 任务编排与阶段指标：`apps/desktop/src-tauri/src/task_manager.rs`
- ASR 常驻与分段 payload：`apps/desktop/src-tauri/src/asr_service.rs`
- LLM 请求/响应与 payload：`apps/desktop/src-tauri/src/llm.rs`
- Debug 开关与 payload 落盘策略：`apps/desktop/src-tauri/src/debug_log.rs`
- 提示词模板内置默认值 + 落盘读写：`apps/desktop/src-tauri/src/templates.rs`
- UI 设置页（模板编辑、导入/导出、rewrite 开关）：`apps/desktop/src/screens/SettingsScreen.tsx`
- 数据目录解析：`apps/desktop/src-tauri/src/data_dir.rs`
- Windows 栈配置：`apps/desktop/src-tauri/.cargo/config.toml`
- Tauri release 资源：`apps/desktop/src-tauri/Cargo.toml`（`custom-protocol`）
- 运行时数据（Windows）：`tmp/typevoice-data/metrics.jsonl`、`tmp/typevoice-data/history.sqlite3`、`tmp/typevoice-data/debug/`
- 运行时模板文件（若存在则覆盖内置默认）：`tmp/typevoice-data/templates.json`

### 关键命令

VERIFIED（WSL/Windows 均可执行；Windows 建议用 PowerShell 执行）

- Windows 编译 release：
- `cd D:\\Projects\\TypeVoice\\apps\\desktop && npm run tauri build`
- Windows 直接运行 release（无控制台）：
- `D:\\Projects\\TypeVoice\\apps\\desktop\\src-tauri\\target\\release\\typevoice-desktop.exe`
- 查询阶段耗时（示例）：
- `jq -r 'select(.type==\"task_perf\") | {task_id, audio_seconds, preprocess_ms, asr_roundtrip_ms, rewrite_ms} | @json' tmp/typevoice-data/metrics.jsonl | tail`
- 找 debug payload 事件（要求 debug verbose 已开）：
- `jq -r 'select(.type|startswith(\"debug_\")) | [.type,.task_id,.payload_path,.note] | @tsv' tmp/typevoice-data/metrics.jsonl | tail`
- 查看当前落盘模板（若存在）：
- `test -f tmp/typevoice-data/templates.json && jq -r '.[] | {id,name} | @json' tmp/typevoice-data/templates.json || echo \"templates.json missing (using built-in defaults)\"`
- 查看当前 `clarify` 的 system prompt（若存在）：
- `test -f tmp/typevoice-data/templates.json && jq -r '.[] | select(.id==\"clarify\") | .system_prompt' tmp/typevoice-data/templates.json || echo \"templates.json missing (using built-in defaults)\"`
- Gate（repo root）：
- `./.venv/bin/python scripts/verify_quick.py`
- `./.venv/bin/python scripts/verify_full.py`

### 关键约束

VERIFIED（来自冻结规格与实现约束）

- ASR 必须使用 CUDA，不允许 CPU 降级（见 `docs/base-spec-v0.1.md`、`docs/verification-v0.1.md`）。
- 取消需要快速（<=300ms），并在任意阶段都可取消（见 `docs/verification-v0.1.md`）。
- Debug 完整 payload 仅在 debug 开关开启时落盘（隐私/可观测性的折中），默认不记录。
- Windows release（GUI 子系统）不要依赖 stdout/stderr 做诊断，优先落盘到 data dir（见 `docs/memory/PITFALLS.md`）。

## 风险与坑（指向 PITFALLS）

VERIFIED

- Windows release 闪退 `0xc00000fd`（栈溢出）可能复发：若移除/覆盖 `apps/desktop/src-tauri/.cargo/config.toml`，需优先回归验证。见 `docs/memory/PITFALLS.md`。
- Windows GUI 子系统无控制台导致“看不到日志/重定向为空”，排障必须走文件落盘；必要时使用 `safe_eprintln!`。见 `docs/memory/PITFALLS.md`。
- 若忘记开启 `TYPEVOICE_DEBUG_VERBOSE`，就无法从日志拿到“分片转录/LLM 原文”。这是预期行为但容易误判为“日志没实现”。见本文件 Now。
- 默认模板更新“看起来没生效”的高概率原因：数据目录存在 `templates.json` 覆盖了内置默认（见本文件 Now/Next；该条是行为特性，不是 bug）。

## 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- Windows 原生 Gate（M12）是否跑通并留档（真源：`docs/tasks-v0.1.md`）。
  - 推荐验证：在 Windows 上跑 `scripts/windows/windows_gate.ps1`，将结果写入 `docs/memory/CONTINUITY.md` 的 Done。
- “每段音频分片的耗时”是否需要精确记录：
  - 当前协议只提供 segments 文本/起止时间，整体 ASR wall time 在 `task_perf.asr_roundtrip_ms`；
  - 推荐验证：检查 `asr_runner/runner.py` 是否能输出 per-segment 处理耗时；若不能，需要设计扩展字段并补测试。
- 默认提示词模板已更新后，是否需要对“已有用户的 `templates.json`”做迁移/提示：
  - 推荐验证：在目标机器上确认是否已存在 `templates.json`，以及 UI 是否能一键导入/覆盖为新模板；如果需要迁移，需先定义“迁移触发条件/回滚策略/对用户已有自定义的处理方式”再动手。
