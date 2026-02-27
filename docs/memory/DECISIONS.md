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

- 决策：热键路径采用“按下即时采集 + 录音会话 ID 注入”，禁止任务开始后再回补窗口截图。
- 依据：窗口在长按/多阶段时会变化，晚采样会导致上下文与真实意图不一致。
- 执行：`recording_session_id` 在热键事件时产生并在 `start_transcribe...` 阶段强制携带。

## 录音会话生命周期清理

- 决策：把上下文清理从“按时间保留（60 秒 TTL）”改为“事务边界清理”：热键按下时打开会话，任务开始时绑定会话并注入上下文，任务结束（完成/取消/失败）时统一清理。
- 依据：会话天然定义了事务范围，避免上下文在中途被兜底清理导致热键上下文与任务生命周期失配。
- 执行：新增 `recording_session_id` 在事件与命令链路中透传，`TaskManager` 按 `task_id` 统一清理未消费会话；命令侧在启动前失败时主动回退该会话。

## 重放式配置策略

- 决策：关键配置以 `settings.json` / `templates.json` 为单一真源，禁止 UI 和热键路径并存各自维护副本参数。
- 依据：副本参数会在事件闭包生命周期产生漂移，导致行为不一致。
- 执行：重度依赖 settings 读链路，入口参数仅传递业务意图而非可变配置快照。

## 文档更新策略

- 决策：发生影响后续工作的新增事实时，必须立刻更新 `docs/` 与 `docs/memory/*`，不得延后。
- 依据：文档复用场景下，延后会导致跨会话恢复时使用过时约束。
- 执行：以 `docs/memory/CONTINUITY.md`（状态）与对应文件（SPEC/DECISIONS/PITFALLS/USER_PREFS）同步记录。

## 严格文档执行的零偏移禁制

- 决策：当用户声明“严格按文档执行”时，执行流程只允许走文档命令原文与文档内明示修复路径，禁止任何自行扩展。
- 依据：历史偏移行为会让“按文档执行”失真，导致用户无法复现与信任执行链路。
- 执行：失败处理固定为“原样回传错误并停止 -> 仅执行文档明示修复 -> 重跑同一命令”；文档未写步骤时必须等待用户指令。

## ASR 状态口径与清单文件处理

- 决策：`asr_model_status` 的模型来源以配置解析链路为准，使用 `pipeline::resolve_asr_model_id` 对齐实际运行模型；`manifest.json` 缺失不再作为 ASR 可用性的致命错误。
- 依据：当前实现的转写依赖模型可加载能力，不依赖 `manifest.json` 本身；该文件仅用于完整性可追溯性。
- 执行：`manifest.json_missing` 保留为告警信息返回，且 UI 应显示“可用但建议补齐 manifest.json”。

## ASR 预处理参数不做缺省回退

- 决策：预处理阶段的行为（静音裁剪开关与参数）仅使用 `settings.json` 中显式配置，不再对字段缺失或解析失败进行额外回退；缺失项由默认值体现，settings 解析失败按现有 fail-fast 模式返回错误。
- 依据：用户要求不引入兼容/回退路径，统一从 settings 作为单一真源驱动 ASR 预处理行为。
- 执行：`start_task` 链路中使用 strict 读取设置并记录 `E_CMD_RESOLVE_ASR_PREPROCESS` 失败码。

## 任务入口统一为 start_task

- 决策：对前端公开单一启动命令 `start_task(req)`，由 `record_mode` 区分输入模式；不再依赖多套 `start_transcribe_*` 命令组合。
- 依据：减少 UI/热键/fixture 并行路径导致的状态机分叉与诊断口径不一致。
- 执行：主 UI 采用 `start_backend_recording` + `stop_backend_recording` 获取 `recording_asset_id`，随后统一调用 `start_task`；后端新增 `CMD.start_task` 统一埋点与失败码。

## 热键注册改为作用域化注销

- 决策：禁止 `unregister_all` 清空全局快捷键，改为仅注销本模块已注册快捷键列表。
- 依据：避免后续多模块共享全局快捷键时互相覆盖。
- 执行：`HotkeyManager` 新增 `registered` 列表并在每次应用设置时按列表逐个注销与重建。

## 未消费录音会话显式回收

- 决策：允许前端在“录音失败/启动任务失败/卸载”等场景主动回收 `recording_session_id`。
- 依据：热键按下先建会话，若后续未进入任务则必须显式清理避免悬挂。
- 执行：新增 `abort_recording_session` 命令，前端在异常分支与卸载清理分支调用。

## 录音输入改为资产句柄

- 决策：任务启动链路统一传输录音资产句柄（`recording_asset_id`），不再保留 `recording_bytes` 输入模式。
- 依据：保持命令语义稳定，降低跨层数据搬运和重复序列化路径。
- 执行：`stop_backend_recording` 返回 `recording_asset_id`，随后统一由 `start_task(record_mode=recording_asset)` 进入任务编排。

## 录音资产生命周期托管

- 决策：`recording_asset_id` 由后端注册并托管，`start_task` 禁止接收裸 `audio_path`。
- 依据：防止启动失败误删外部文件，并避免 stop/start 之间中间产物失管。
- 执行：后端维护录音资产表与短时租约，过期自动回收；`start_task` 仅消费资产句柄。

## 录音改为后端进程托管

- 决策：主录音链路改为后端命令托管，不再由前端 `MediaRecorder` 直接采集与拼装音频。
- 依据：统一录音与任务状态机边界，减少前端持有音频生命周期导致的执行路径分叉。
- 执行：新增 `start_backend_recording` / `stop_backend_recording` / `abort_backend_recording`，由后端调用 FFmpeg(dshow) 管理录音进程；停止命令只结束录音并回传产物句柄。

## 更正记录（任务启动与字节模式描述）

- 更正：此前条目“`MainScreen` 已切换调用 `start_task`”已不再准确。
- 当前口径：主 UI 录音链路通过 `start_backend_recording` + `stop_backend_recording` 停止采集，再调用 `start_task(record_mode=recording_asset)` 进入统一任务编排。
- 更正：此前条目“`MainScreen` 直接以 `audioBytes` 调用 `start_task(record_mode=recording_bytes)`”已失效，`recording_bytes` 模式已移除。

## 更正记录（配置读取错误码说明）

- 更正：此前条目中提到的旧录音转写命令链路已删除，不再作为 settings strict 读取的执行入口。
- 当前口径：settings strict 读取路径由 `start_task`（及历史保留的非命令内部路径）承接，错误码仍维持 `E_CMD_RESOLVE_ASR_PREPROCESS` 族。

## 录音输入策略建模（follow_default|fixed_device|auto_select）

- 决策：录音设备选择改为显式策略模型，持久化“策略 + 用户选择”，不再把一次探测得到的 dshow spec 当作唯一真相。
- 依据：`audio=default` 在部分 Windows 设备/驱动组合下不可解析；同时用户对“跟随系统默认”和“固定设备”是两种不同预期，不能混在同一字段里。
- 执行：
  - `settings.json` 新增 `record_input_strategy`、`record_follow_default_role`、`record_fixed_endpoint_id`、`record_fixed_friendly_name`。
  - `follow_default` 默认 role 固定为 `communications`（语音输入场景优先）。
  - 新增 `record_last_working_*` 缓存字段，仅用于容错回退，不回写为用户配置。
  - `start_backend_recording` 每次启动前按策略解析输入：
    - `fixed_device`: 固定设备 -> 系统默认 -> 自动选择
    - `follow_default`: 系统默认 -> last_working -> 自动选择
    - `auto_select`: 枚举 + probe 选择可用设备

## 可调试性失败展示契约

- 决策：前端失败提示必须展示 `error_code + 摘要 + 建议动作`，禁止仅展示泛化 `ERROR` / `CANCEL FAILED`。
- 依据：仅有泛化失败文案会让用户无法直接定位根因，违背“可观测”目标。
- 执行：`MainScreen` 统一错误归一化，失败事件与命令异常都生成可见诊断行。

## ASR 冷启动错误码保真

- 决策：ASR runner 在 ready 前返回 `ok=false/error.code` 时，后端直接透传该错误码并附带 stderr tail 诊断。
- 依据：此前会退化为 `E_ASR_READY_EOF`，导致真正根因（如 `E_MODEL_LOAD_FAILED`）丢失。
- 执行：`asr_service::restart` 在 ready 读取循环识别结构化错误行；spawn 改为 `stderr` 可读并限长记录。

## 可调试性自动化门禁

- 决策：把可调试性契约并入 `verify_quick` / `verify_full`，作为固定 gate，而非人工抽查。
- 依据：可调试性回归（吞错、日志损坏）具有高隐蔽性，靠手测难以稳定捕获。
- 执行：
  - Rust 单测：`trace` 并发 JSONL 可解析、ASR 错误行解析保真、内部失败事件结构完整。
  - 脚本 gate：`verify_quick/full` 固定执行上述契约测试，并要求 ASR 失败包含结构化错误码。

## TaskManager 依赖注入缝隙

- 决策：`TaskManager` 外部依赖改为可注入，不在构造器里硬编码不可替换实现。
- 依据：核心编排单测需要替换 ASR/上下文/FFmpeg/模板/持久化/指标等外部系统。
- 执行：新增 `AsrClient`、`ContextCollector`、`TaskManagerDeps`，`TaskManager::new()` 只负责默认装配。

## 前端运行时端口约束

- 决策：屏幕组件不得直接依赖 `@tauri-apps/api` 或 `window` 定时器。
- 依据：业务状态测试需要脱离 Tauri 运行时，避免每次组件测试都 mock IPC/事件总线。
- 执行：新增 `runtimePorts.ts`（`TauriGateway`/`TimerPort`/`ClipboardPort`），`MainScreen`/`SettingsScreen`/`HistoryScreen`/`OverlayApp` 统一通过端口访问平台能力。

## 导出阶段自动粘贴（非快捷键）

- 决策：导出阶段新增 `export_text`，统一执行“复制 + 自动粘贴”；自动粘贴默认开启，可在设置中关闭。
- 依据：用户要求在 Linux 与 Windows 下自动粘贴，且粘贴动作不得通过快捷键模拟。
- 执行：
  - `settings.json` 新增 `auto_paste_enabled`（默认 `true`）。
  - Windows 使用 `SendInput + KEYEVENTF_UNICODE` 直接提交 Unicode 文本输入，并收敛为“当前前台线程焦点控件单路径执行”：不使用 `last_external_hwnd` hint，不做目标窗口回退。
  - Windows 导出时若 overlay 显示，前端先隐藏 overlay 再调用 `export_text`，避免 overlay 抢前台焦点。
  - Windows 目标判定增加“自进程拒绝”：若前台窗口/焦点窗口属于 TypeVoice，返回 `E_EXPORT_TARGET_UNAVAILABLE`，禁止上报“自动输入成功”。
  - Linux 使用 AT-SPI 在焦点可编辑对象执行文本写入（不走快捷键）。
  - 自动粘贴失败返回结构化错误码并在 UI 显示，不静默吞错。

## ASR 提供方切换与运行边界

- 决策：ASR 运行时按 `settings.asr_provider` 选择 `local|remote`；默认 `local`，`remote` 不启动/保活本地 runner。
- 依据：用户要求“可选本地或云端 ASR”，并要求模块分离、避免远程路径和本地进程生命周期耦合。
- 执行：
  - `TaskManager` 在 `Transcribe` 阶段按 provider 分支。
  - `warmup/restart` 仅在 `local` 生效；切换到 `remote` 时执行本地 runner `kill_best_effort`。
  - 指标新增 `asr_provider`、`remote_asr_slice_count`、`remote_asr_concurrency_used`。

## Remote ASR 协议与并发切片

- 决策：Remote ASR 采用固定协议：`POST /transcribe` + Bearer 鉴权 + multipart `file`（可选 `model`），解析 JSON `text`。
- 依据：用户已给出 Whisper 风格兼容协议，要求可配置地址、key 与模型名。
- 执行：
  - 新增 `remote_asr` 模块，独立承担 keyring、请求发送、错误码与文本合并。
  - 音频按切片并发请求，默认 60s 切片 + 0.5s overlap，`remote_asr_concurrency` 限定 `1..16`（默认 `4`）。
  - 片段返回后按序合并并做重叠去重，失败返回结构化错误码。

## Runner 退出状态实例化

- 决策：ASR runner 禁止使用进程级全局退出标记。
- 依据：全局可变状态会污染同进程重复测试并降低可预测性。
- 执行：`runner.py` 引入 `RunnerRuntime` 实例管理退出状态，signal handler 写实例字段，不再使用全局 `_should_exit`。
