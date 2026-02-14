# CONTINUITY（外置记忆）

- 本文件记录当前可执行进度与工作集，必须可直接恢复下一步工作，不包含历史复盘。
- 优先以 `docs/` 中冻结文档为真源，`docs/memory` 仅保留当前状态快照。

## 当前有效目标

- 保持 MVP 核心能力的稳定可用性：录音、预处理、转录、可选改写、复制、可观测。
- 继续压实可复用的上下文与错误诊断链路，支持热键与 UI 两条路径行为一致。

## 当前状态（进行中）

### 已确认

- 结构化诊断链路已启用：`TYPEVOICE_DATA_DIR/trace.jsonl` 为主日志入口，关键路径记录稳定的 `step_id` 与 `code`。
- 热键上下文采集已改为“录音会话（recording_session_id）”路径：按热键在按下瞬间创建会话并缓存上下文快照，开始转写时消费该会话并在任务收尾统一清理，剔除“capture_id 60 秒有效期”分支。
- 上下文采集与改写参数链路按设置主源，支持配置缺失即失败（fail-fast），不引入静默兜底。
- `docs/` 文档索引与命名清洗已完成，历史版本后缀已移除。
- 可复用的执行索引文件已新增：`docs/index.md`。
- 词汇表重写入口已完成：新增 `rewrite_glossary` 配置持久化与 rewrite content 透传，默认提示词已加入 `### GLOSSARY` 约束说明。
- [VERIFIED] Windows 侧补齐“最新源码构建并启动”流程：新增 `scripts/windows/run-latest.ps1`，并在 `docs/windows-dev.md` / `docs/windows-gate.md` 固化一键执行路径。
- [VERIFIED] ASR 预处理新增可配置项已接通：新增 `asr_preprocess_silence_*` 设置字段，`Settings` 与 `SettingsPatch` 持久化，并在 `SettingsScreen` 新增 `PREPROCESS` 开关/参数面板（静音裁剪开关、阈值、首尾静音时长）。
- [VERIFIED] 预处理配置已纳入运行链路：`task_manager::StartOpts`、`StartOpts` 组装、FFmpeg 预处理阶段与日志指标（`task_perf`）均记录 `asr_preprocess_*`，用于比对配置对 ASR 时延的影响。
- [UNCONFIRMED] 任务启动入口已统一：新增 `start_task(req)` 命令并删除旧的多入口转写命令实现；待校验路径：`npm run tauri dev` + 热键/UI 双链路 smoke。
- [UNCONFIRMED] 更正：主 UI 录音链路已收敛为 `start_backend_recording` -> `stop_backend_recording` -> `start_task(record_mode=recording_asset)`；任务启动只经 `start_task`。
- [UNCONFIRMED] 录音中间产物已切换为后端托管 `recording_asset_id` 语义：任务启动不再接收裸文件路径，资产由后端租约回收。
- [UNCONFIRMED] 后端同步清理了旧转写命令实现，当前命令面仅保留统一任务链路。
- [UNCONFIRMED] 热键注册已改为“作用域化注销”语义：`HotkeyManager` 仅注销自身曾注册快捷键，不再调用 `unregister_all`；待校验路径：热键重复保存设置后仍可触发，且不影响其他 scope。
- [UNCONFIRMED] 热键会话清理链路已补齐：新增 `abort_recording_session` 命令，前端在录音失败/转写启动失败/组件卸载时会回收未消费 `recording_session_id`；待校验路径：trace 中无悬挂 session。
- [UNCONFIRMED] 上下文窗口采样语义已向“前台窗口即时采样”收敛：hotkey 与任务内上下文均优先使用 `foreground_window_*` 路径；待校验路径：同 task_id 下两入口截图来源一致。
- [UNCONFIRMED] 录音停止诊断链路已增强：`start_backend_recording` 增加 ffmpeg 早退探测（避免“启动已失败但在 stop 才暴露”），`stop_backend_recording` 在失败时附带 stderr 末行，前端停止失败提示改为展示真实错误提示（不再固定 `STOP FAILED`）；待校验路径：Windows 下复现一次录音设备异常并确认错误文案含 `E_RECORD_*` 与 stderr 线索。
- [VERIFIED] Windows 侧已定位 `Recording failed` 根因为 dshow 输入规格 `audio=default` 在当前设备集上不可解析；`ffmpeg` 直接探测显示 `Error opening input files: I/O error`。固定为设备 `Alternative name`（如 `audio="@device_cm_{...}\\wave_{...}"`）后可稳定录制，且不受蓝牙耳机连接导致默认设备切换的影响。
- [UNCONFIRMED] 后端录音输入已改为“自动探测并固化”机制：当 `record_input_spec` 为空时，后端使用 `ffmpeg -list_devices` 枚举 dshow 音频设备并按可录制探测选择可用设备，将选中的 `audio="@device_cm_{...}\\wave_{...}"` 回写到 `settings.json`；待校验路径：清空 `record_input_spec` 后首次录音自动成功，随后蓝牙设备连接/断开不改变已固化输入源。
- [UNCONFIRMED] 已修正自动探测规格转义错误：`Process::Command` 场景下不再给 dshow 设备名附加内层引号（避免把引号当作设备名字面量导致 `I/O error`），并在读取已配置 `record_input_spec` 时自动归一化 `audio=\"...\"` 旧格式。
- [VERIFIED] 可调试性链路补强完成：前端失败提示不再只显示泛化 `ERROR`，已展示 `error_code + 摘要 + 建议动作`；`cancel_task` 失败返回显式带 `E_CMD_CANCEL`。
- [VERIFIED] ASR 冷启动错误码保真已修复：runner 在 ready 前返回 `ok=false/error.code` 时，后端直接透传真实错误码并记录 stderr tail，不再退化为 `E_ASR_READY_EOF`。
- [VERIFIED] 任务启动 runtime 创建失败已补发终态：`tokio runtime build Err` 分支会发 `task_event.failed(E_INTERNAL)`，避免 UI 卡住无终态。
- [VERIFIED] trace 并发写入已加全局互斥，新增并发 JSONL 可解析测试 `trace::tests::concurrent_emit_keeps_jsonl_lines_parseable`。
- [VERIFIED] gate 已纳入可调试性自动化：`verify_quick/full` 新增 Rust 可调试性契约测试步骤，并要求 ASR 失败时必须有结构化 `error.code`。
- [VERIFIED] 本仓已完成回归：`cargo check --locked`、`npm run build`、`pytest -q tests`、`verify_quick.py`、`verify_full.py` 全部通过。
- [VERIFIED] 2026-02-14：已清理错误路径 `/home/atticusdeng/Projects/TypeVoice` 下误拉取产物（`.venv`、`models`、`apps/desktop/node_modules`、toolchain 可执行）；随后按文档在 `D:\Projects\TypeVoice` 执行 `.\scripts\windows\run-latest.ps1`，成功拉起 Windows runtime（`typevoice-desktop.exe`, PID `38500`）。
- [VERIFIED] `TaskManager` 已引入可注入依赖缝隙：新增 `AsrClient`、`ContextCollector` 与 `TaskManagerDeps`，核心编排不再硬编码外部依赖构造。
- [VERIFIED] 前端已接入运行时端口层：新增 `src/infra/runtimePorts.ts`，`MainScreen`/`SettingsScreen`/`HistoryScreen`/`OverlayApp` 通过 `TauriGateway`/`TimerPort`/`ClipboardPort` 访问平台能力。
- [VERIFIED] 诊断逻辑已从 UI 抽离：新增 `src/domain/diagnostic.ts`，`MainScreen` 仅消费纯函数输出，降低组件内业务逻辑耦合。
- [VERIFIED] `asr_runner/runner.py` 已移除全局 `_should_exit`，改为 `RunnerRuntime` 实例状态；并引入 `ProbePort`/`ModelPort` 注入缝隙。
- [VERIFIED] 本轮本机回归结果：`cargo test -q`（通过）、`npm run build`（通过）、`./.venv/bin/python -m pytest -q tests`（通过）。
- [VERIFIED] `verify_quick.py` 在当前环境失败：缺少本地模型目录 `models/Qwen3-ASR-0.6B`；失败原因为环境资产缺失而非编译错误。

### 当前工作集

- 需要在下一步优先验证：
  - 热键触发链路的 rewrite 一致性（`rewrite_enabled/template_id` 与设置一致）。
  - 关键 Windows-only 改动后的本机 `Windows` `npm run tauri dev` 与 `quick/full` 可回归（当前 Linux 侧已通过）。
- [UNCONFIRMED] “后端录音替代前端 `MediaRecorder`”已接通：`MainScreen` 调用 `start_backend_recording` / `stop_backend_recording` / `abort_backend_recording`，录音由后端 FFmpeg 进程托管；待校验路径：Windows 默认麦克风 dshow 输入可用性。
- [UNCONFIRMED] `start_task(recording_bytes)` 已从任务入口移除；当前统一输入模式为 `recording_asset|fixture`。
- 进行中更新点：`docs/memory` 由复盘内容转为“当前事实清单”。
- ASR 设置页状态来源修正：状态检查已改为按 `settings` 解析后的实际 ASR 模型来源（本地路径或远端模型 id）执行，不再固定指向仓库默认目录。
- 完整性告警策略调整：`manifest.json_missing` 仍可见，但不再阻断 ASR 可用性，状态面板会明确标注为“可用但未做完整性清单校验”。

## 下一步建议

- 在 Windows 下补一次完整闭环验证：热键按下->开始录音->停止/取消->转写完成/失败，确认同一 `recording_session_id` 仅在任务结束时清理一次。
- 在 Windows 会话里补齐一次“热键录音 -> 转写 -> 改写 -> persist -> copy”闭环，并将观察结果回写：
  - 正常：同一 `task_id` 链路内 `CMD.start...`、`TASK.rewrite_effective`、`task_done` 一致。
  - 异常：将复现条件与处理思路写入 `docs/memory/PITFALLS.md`。
- 在本仓执行 `git status` 与 `./.venv/bin/python scripts/verify_full.py` 前后对照文档约定。
- 避免改动 `docs/memory` 以外文档时引入重复事实，若新增约束立即写回对应文件。

## 变更后固定执行流程

- 所有代码修复/功能更新完成后，Windows 侧必须按以下顺序执行：
  1. 同步源码（拉取当前分支最新提交）。
  2. 重新编译（推荐按项目文档链路执行）。
  3. 以 Debug 模式启动进程并复测相关链路。

## 关键文件（当前可直接核验）

- `apps/desktop/src-tauri/src/trace.rs`：trace/diagnostic 主流程。
- `apps/desktop/src-tauri/src/context_capture.rs`、`apps/desktop/src-tauri/src/context_capture_windows.rs`：上下文采集。
- `apps/desktop/src-tauri/src/lib.rs`、`pipeline.rs`、`settings.rs`、`templates.rs`：流程入口与配置。
- `scripts/verify_quick.py`、`scripts/verify_full.py`：验收闭环。
- `docs/index.md`：文档导航入口。
- [ASR 预处理路径] `apps/desktop/src-tauri/src/pipeline.rs`：
  - `PreprocessConfig`、`build_ffmpeg_preprocess_args`、`clamp_preprocess_config`。
  - `run_audio_pipeline_with_task_id`、`run_fixture_pipeline`、`preprocess_ffmpeg(_cancellable)` 增加配置入参。
- [ASR 配置链路] `apps/desktop/src-tauri/src/settings.rs`、`lib.rs`、`task_manager.rs`、`apps/desktop/src/screens/SettingsScreen.tsx`、`apps/desktop/src/types.ts`。

## 关键命令（仅当前可复用）

- `./.venv/bin/python scripts/verify_quick.py`
- `./.venv/bin/python scripts/verify_full.py`
- Windows 环境文档命令：
  - `Set-Location <repo>/apps/desktop`
  - `npm run tauri dev`
- WSL 下触发 Windows 文档命令必须保持文档命令语义不变，仅调整调用链环境。

## 关键约束

- 不增加用户交互类“发送预览/确认”流程；上下文采集与发送保持自动化。
- 所有新增行为变更须同时落在 `docs/*` 与对应 `docs/memory` 文件里。
- 任何多路径并发日志结论必须按 `task_id` 聚合，不按尾部单行推断可用性。
