# DECISIONS（关键决策与理由）

- 本文件用于记录稳定行为的关键取舍，配合 `docs/` 中冻结文档形成决策依据。
- 仅保留当前有效决策，不保留历史辩论与已失效实现路径。

## 诊断优先于猜测

- 决策：失败定位以落盘结构化日志为优先依据，不依赖控制台输出。
- 依据：Windows release 缺少稳定控制台，错误链路必须包含 `step_id`、`code`、`error_chain`、`backtrace`。
- 执行：以 `apps/desktop/src-tauri/src/trace.rs` 为主入口，错误由日志完成归因。

## Windows-only 改动的最小验证门

- 决策：触及 `windows` 分支、窗口截图、热键路径的提交，必须在目标 Windows 环境执行一次验证命令链路。
- 依据：Linux/WSL 无法覆盖 `#[cfg(windows)]` 条件下的编译与运行差异。
- 执行：提交前补跑 `scripts/windows/windows_compile_gate.ps1` 与对应 Windows 运行流程验证。

## ASR/LLM 工具链与路径优先级

- 决策：Windows 调试运行时以仓库 toolchain 为准，缺失 `ffmpeg` 时按项目约定脚本修复，不绕过到不受控环境。
- 依据：路径漂移会在前期 preflight 形成确定性失败，难以与 ASR 运行时问题区分。
- 执行：`apps/desktop/src-tauri/src/pipeline.rs` 与相关工具链入口保持单一解析顺序。

## 运行时上下文采集策略

- 决策：热键路径采用“按下即时采集 + 一次性 capture_id 注入”，禁止任务开始后再回补窗口截图。
- 依据：窗口在长按/多阶段时会变化，晚采样会导致上下文与真实意图不一致。
- 执行：`capture_id` 在热键事件时产生并在 `start_transcribe...` 阶段强制携带。

## 重放式配置策略

- 决策：关键配置以 `settings.json` / `templates.json` 为单一真源，禁止 UI 和热键路径并存各自维护副本参数。
- 依据：副本参数会在事件闭包生命周期产生漂移，导致行为不一致。
- 执行：重度依赖 settings 读链路，入口参数仅传递业务意图而非可变配置快照。

## 文档更新策略

- 决策：发生影响后续工作的新增事实时，必须立刻更新 `docs/` 与 `docs/memory/*`，不得延后。
- 依据：文档复用场景下，延后会导致跨会话恢复时使用过时约束。
- 执行：以 `docs/memory/CONTINUITY.md`（状态）与对应文件（SPEC/DECISIONS/PITFALLS/USER_PREFS）同步记录。
