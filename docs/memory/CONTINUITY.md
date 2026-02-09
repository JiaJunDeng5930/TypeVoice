# CONTINUITY（外置记忆）

说明：

- 本文件记录“当前进度与工作集”，用于跨回合恢复上下文。
- 内容必须区分 `VERIFIED`（能用代码/文档/命令复核）与 `UNCONFIRMED`（推断/未验证）。
- 冻结规格的真源在 `docs/*.md`；本文件只做“可执行的接手摘要”，避免复述导致漂移（VERIFIED：`docs/memory/SPEC.md`）。

## 当前有效目标（与 SPEC 对齐）

VERIFIED（截至 2026-02-09）

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

### Now

VERIFIED（当前现状）

- Windows release 版本可稳定运行，数据目录为 `D:\\Projects\\TypeVoice\\tmp\\typevoice-data`（等价于默认 `tmp/typevoice-data`）。
- 最新一次任务的 `metrics.jsonl` 中只有 `task_event/task_perf`，没有 `debug_*` 事件；且 `debug/` 目录不存在：说明当次运行时未开启 `TYPEVOICE_DEBUG_VERBOSE`，因此“分片转录/LLM 请求与回复原文”没有被记录下来（这是预期行为，不是 bug）。

### Next

VERIFIED（下一步推进顺序，零上下文 agent 可直接做）

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
- 数据目录解析：`apps/desktop/src-tauri/src/data_dir.rs`
- Windows 栈配置：`apps/desktop/src-tauri/.cargo/config.toml`
- Tauri release 资源：`apps/desktop/src-tauri/Cargo.toml`（`custom-protocol`）
- 运行时数据（Windows）：`tmp/typevoice-data/metrics.jsonl`、`tmp/typevoice-data/history.sqlite3`、`tmp/typevoice-data/debug/`

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

## 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- Windows 原生 Gate（M12）是否跑通并留档（真源：`docs/tasks-v0.1.md`）。
  - 推荐验证：在 Windows 上跑 `scripts/windows/windows_gate.ps1`，将结果写入 `docs/memory/CONTINUITY.md` 的 Done。
- “每段音频分片的耗时”是否需要精确记录：
  - 当前协议只提供 segments 文本/起止时间，整体 ASR wall time 在 `task_perf.asr_roundtrip_ms`；
  - 推荐验证：检查 `asr_runner/runner.py` 是否能输出 per-segment 处理耗时；若不能，需要设计扩展字段并补测试。

