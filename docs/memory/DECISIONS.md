# DECISIONS（关键决策与理由）

- 本文件用于记录稳定行为的关键取舍，配合 `docs/` 中冻结文档形成决策依据。
- 仅保留当前有效决策，不保留历史辩论与已失效实现路径。

## 诊断优先于猜测

- 决策：失败定位以落盘结构化日志为优先依据，不依赖控制台输出。
- 依据：Windows release 缺少稳定控制台，错误链路必须包含 `step_id`、`code`、`error_chain`、`backtrace`。
- 执行：以 `apps/desktop/src-tauri/src/obs/trace.rs` 与 `apps/desktop/src-tauri/src/obs/writer.rs` 为主入口，错误由结构化日志完成归因。

## Dependabot 限流与分组策略

- 决策：仓库保留 Dependabot，但按生态聚合更新，并将版本更新 PR 数量限制到每生态同时最多 `1` 个。
- 依据：默认配置会在一次检查中产生多个分散 PR，增加分支噪音与评审负担；本仓当前更适合“低噪音、可持续处理”的依赖更新节奏。
- 执行：`.github/dependabot.yml` 对 `npm`、`cargo`、`pip` 统一设置 `open-pull-requests-limit: 1`，并分别为 `version-updates` / `security-updates` 配置 `groups` 聚合规则。

## 前端错误统一落盘

- 决策：前端可见错误必须以结构化事件回传后端并落盘到 `trace.jsonl`，不能只依赖 UI 文案或 overlay 状态。
- 依据：仅有 `CMD.overlay_set_state(status=ERROR)` 无法还原真实失败原因，且前端 fallback 文案会造成误导。
- 执行：新增 `ui_log_event` 命令，统一记录 `UI.toast` / `UI.diagnostic` / `UI.invoke_error` 事件，并要求包含稳定 `error.code` 与上下文字段。

## 热键失败码显式化

- 决策：热键失败场景必须输出准确错误码，禁止把“任务占用/事件不完整”误记为 `E_HOTKEY_CAPTURE`。
- 依据：`E_HOTKEY_CAPTURE` 应仅表达窗口捕获失败；混用会直接误导排障方向。
- 执行：后端在热键占用分支输出 `E_TASK_ALREADY_ACTIVE`，前端缺字段 fallback 改为 `E_HOTKEY_EVENT_INCOMPLETE`，并在 `Hotkeys` stage 记录拒绝/打开失败/事件发射失败三类结构化错误事件。

## Windows-only 改动的最小验证门

- 决策：触及 `windows` 分支、窗口截图、热键路径的提交，必须在目标 Windows 环境执行一次验证命令链路。
- 依据：Linux/WSL 无法覆盖 `#[cfg(windows)]` 条件下的编译与运行差异。
- 执行：提交前补跑 `scripts/windows/windows_compile_gate.ps1` 与对应 Windows 运行流程验证。

## ASR/LLM 工具链与路径优先级

- 决策：Windows 调试运行时以仓库 toolchain 为准，缺失 `ffmpeg` 时按项目约定脚本修复，不绕过到不受控环境。
- 依据：路径漂移会在前期 preflight 形成确定性失败，难以与 ASR 运行时问题区分。
- 执行：`apps/desktop/src-tauri/src/pipeline.rs` 与相关工具链入口保持单一解析顺序。

## Windows 子进程控制台窗口策略

- 决策：Windows release 下所有外部命令子进程统一使用 `CREATE_NO_WINDOW` 创建标志，禁止弹出控制台窗口。
- 依据：`python.exe` / `ffmpeg.exe` / `ffprobe.exe` 等为 console 子系统可执行文件；在 GUI 主进程下直接 `spawn/output/status` 会出现黑框，并可能被用户手动关闭导致管道断开（如 ASR 写入 `os error 232`）。
- 执行：Rust 侧通过 `subprocess::CommandNoConsoleExt` 在关键 `Command` 调用链统一 `.no_console()`；Python runner 内部 `subprocess.check_output`（ffprobe）在 Windows 分支使用 `creationflags=subprocess.CREATE_NO_WINDOW`。

## 运行时上下文采集策略

- 决策：热键路径采用“按下即时采集 + `task_id` 注入”，禁止任务开始后再回补窗口截图。
- 依据：窗口在长按/多阶段时会变化，晚采样会导致上下文与真实意图不一致。
- 执行：`task_id` 在热键按下事件时产生并在 `start_task(req)` 阶段强制携带；`TaskManager` 以 `task_id` 作为唯一生命周期主键消费预采样上下文。

## 改写上下文改为文本-only

- 决策：移除窗口截图、vision 开关和多模态 LLM 请求；改写上下文固定为文本结构。
- 依据：用户明确要求直接去掉截图功能，并保持上下文能力全部可设置。
- 执行：删除 `context_include_prev_window_screenshot`、`llm_supports_vision`、`ContextSnapshot.screenshot` 与 LLM `ImageUrl` 分支；上下文由焦点应用/元素、输入状态、相关文本、可见文本、历史和剪贴板组成。

## 热键预采样上下文生命周期清理

- 决策：将原 `recording_session_id` 生命周期并入 `task_id`，上下文清理从“独立会话清理”收敛为“单任务清理”。
- 依据：同一链路同时维护 session 与 task 两套 ID 会增加状态分叉和补洞清理复杂度。
- 执行：后端维护 `pending_hotkey_contexts(task_id -> context)`；`start_task` 消费该上下文，失败/卸载路径通过 `abort_pending_task(task_id)` 回收未消费上下文。

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

## 未消费热键上下文显式回收

- 决策：允许前端在“录音失败/启动任务失败/卸载”等场景主动回收未消费的 `task_id` 预采样上下文。
- 依据：热键按下已完成上下文采集，若后续未进入任务则必须显式清理避免悬挂。
- 执行：命令更换为 `abort_pending_task`，前端在异常分支与卸载清理分支调用。

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
  - 新增 `RecordInputCacheState`：解析与 probe 在以下时机刷新缓存：
    - `app_startup`
    - `set_settings` / `update_settings`（录音输入相关配置变更）
    - Windows 音频设备变更事件（`IMMNotificationClient`）
  - `start_backend_recording` 热路径只读取缓存，不再执行 dshow 枚举/probe。
  - 缓存命中失败时返回 `E_RECORD_INPUT_CACHE_NOT_READY`，避免在热路径隐式触发慢操作。

## 录音提示时机修正

- 决策：hotkey 触发时不提前显示 `REC`，只在后端录音进程成功拉起后显示 overlay。
- 依据：此前 UI 在 `start_backend_recording` 返回前就显示 `REC`，会误导用户提前开口并放大“前半段丢失”体感。
- 执行：`MainScreen.startRecording` 将 `overlaySet(true, "REC")` 移到 `start_backend_recording` 成功分支。

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

## 日志统一写入架构（异步单写线程）

- 决策：结构化日志统一收敛到 `obs` 模块，`trace.jsonl` 与 `metrics.jsonl` 使用“有界队列 + 单写线程”写入；业务代码只提交结构化事件，不直接处理并发写文件。
- 依据：原实现中 `trace` 与 `metrics` 的并发保障不一致（`trace` 有互斥，`metrics` 无互斥），容易出现 JSONL 粘连、语义分散与维护成本上升。
- 执行：
  - 新增 `apps/desktop/src-tauri/src/obs/`：`schema.rs`（统一事件结构）、`writer.rs`（队列与单写线程）、`trace.rs`、`metrics.rs`、`debug.rs`、`startup.rs`、`panic.rs`。
  - 删除旧模块：`trace.rs`、`metrics.rs`、`debug_log.rs`、`startup_trace.rs`、`panic_log.rs`。
  - `TaskManager` 与 debug artifact 统一使用 `MetricsRecord`，`metrics.jsonl` 强制包含稳定 `type` 与 `ts_ms`。
  - 队列拥塞时不阻塞主流程，记录 `logger_dropped` 指标，确保丢弃可观测。

## 仓库许可证选择（MIT）

- 决策：仓库整体许可证固定为 MIT。
- 依据：项目维护者明确选择“最宽松许可”作为当前对外发布策略。
- 执行：仓库根目录新增 `LICENSE`（MIT 文本），并在包元数据与 README 中同步标注。

## 代码所有权模型（单维护者）

- 决策：`CODEOWNERS` 采用单维护者模式，当前仅维护者本人负责目录评审归属。
- 依据：当前阶段无协同维护者，代码评审责任由维护者独立承担。
- 执行：仓库根目录新增 `CODEOWNERS`，全局路径映射到维护者账号。

## FFmpeg 合规声明固定口径

- 决策：当前内置 FFmpeg 二进制按 GPL 路线声明，仓库持续维护第三方声明入口文件。
- 依据：当前内置二进制构建参数包含 `--enable-gpl` 与 `libx264/libx265`。
- 执行：新增 `THIRD_PARTY_NOTICES.md`，并在 `docs/tech-spec.md` 固化 FFmpeg 版本、来源与许可口径。

## Tauri CSP 显式化

- 决策：前端安全策略由 `csp=null` 调整为显式 CSP 字符串，禁止无策略运行。
- 依据：`null` 配置无法提供 WebView 侧基础脚本/连接来源限制。
- 执行：`apps/desktop/src-tauri/tauri.conf.json` 固化非空 CSP，允许 dev 所需 localhost/ws 与业务 HTTPS 连接。

## Fixtures 可复现下载清单

- 决策：验证音频样本改为“manifest 固定 + 自动下载 + sha256 校验”流程，不再依赖手工准备。
- 依据：`quick/full` 在新环境与 CI 中需要可直接复现，且音频本体不入库。
- 执行：新增 `scripts/fixtures_manifest.json` 与 `scripts/download_fixtures.py`，并在 `verify_quick.py` / `verify_full.py` 运行前自动校验样本完整性。

## FFmpeg 上游签名链校验

- 决策：工具链脚本增加 FFmpeg 上游 release source PGP 验签与签名指纹固定校验。
- 依据：仅有下载包 `sha256` 只能校验完整性，无法验证发布者身份。
- 执行：`scripts/download_ffmpeg_toolchain.sh` 与 `scripts/windows/download_ffmpeg_toolchain.ps1` 先验证 `ffmpeg-devel.asc` 与 source `.asc` 签名，再执行预编译包下载校验。
