# DECISIONS（关键决策与理由）

说明：

- 记录“做了什么关键取舍/为什么”，用于处理冲突与版本漂移。
- 必须区分 `VERIFIED` / `UNCONFIRMED`。

## Windows Release 启动栈溢出：提升主线程栈保留大小

VERIFIED（2026-02-09）

- 背景：Windows Release 版 `typevoice-desktop.exe` 启动后闪退，事件日志显示异常 `0xc00000fd`（STATUS_STACK_OVERFLOW），且复现稳定。
- 决策：为 Windows MSVC 目标设置更大的主线程栈保留大小（8MB），避免启动阶段栈深导致的溢出。
- 方案：在 `apps/desktop/src-tauri/.cargo/config.toml` 添加：
- `rustflags = ["-C", "link-arg=/STACK:8388608"]`
- 取舍：属于“运行时健壮性优先”的工程化修复；不改变业务逻辑，仅改变链接器栈配置。
- 复核方式：检查 PE 头 `SizeOfStackReserve` 为 `0x00800000`；并验证 release 进程稳定运行 >= 30s 且无新的 APPCRASH 事件。

## 统一行尾为 LF（仅源码/配置范围）

VERIFIED（2026-02-09）

- 背景：本仓库采用“WSL 统一编辑、Windows 侧仅用于编译运行验证”的双工作副本模式；若 Windows checkout 发生 CRLF 漂移，容易导致大量文件被误判为变更，并放大增量编译失效的概率（见 `docs/windows-dev-from-wsl-v0.1.md` 中对 Windows 侧编辑与行尾警告的提醒）。
- 决策：新增根目录 `.gitattributes`，对源码/常见配置与脚本文件类型强制 `eol=lf`，避免行尾漂移造成噪声改动与无效重编译。
- 方案：`.gitattributes` 覆盖 `*.rs/*.ts(x)/*.py/*.toml/*.json/*.md/*.ps1/*.sh` 等；并显式约束 `Cargo.lock` 为 LF。
- 复核方式：
  - 在 Windows repo `git pull` 后执行 `git status`，不应出现大面积“仅行尾变化”的改动；
  - 在 WSL repo `git status` 保持干净。

## Rewrite 上下文（ContextPack）：自动附带历史/剪贴板/上一外部窗口截图

VERIFIED（2026-02-10）

- 背景：仅发送 `asr_text` 给 LLM 容易缺少语境；且目标产品形态为“快捷键按下即录、松开即发送”，不希望引入需要用户确认的 UI 预览/勾选。
- 决策：Rewrite 阶段默认自动拼装上下文（最近 N 条历史 + 剪贴板文本 + “TypeVoice 之前的外部前台窗口”截图），无需 UI 操作。
- 截图实现：Windows 上使用 Win32 `GetForegroundWindow` 追踪“上一外部前台窗口”，并使用 `PrintWindow` 抓取窗口像素后编码为 PNG（见 `apps/desktop/src-tauri/src/context_capture_windows.rs`）。
- 调试取舍：开启 debug verbose 时落盘 `llm_request.json` 仍保留结构化请求，但对截图 base64 做脱敏，仅保留 sha/尺寸/字节数，避免敏感像素落盘与文件爆炸（见 `apps/desktop/src-tauri/src/llm.rs`）。

## 减少 dev profile 调试信息以缩短链接时间

VERIFIED（2026-02-09）

- 背景：Windows 侧 `tauri dev` 会频繁触发 Rust debug 构建；在依赖体量较大的情况下（`tauri`/`tokio`/`reqwest`/`rusqlite` 等，见 `apps/desktop/src-tauri/Cargo.toml`），链接阶段往往成为迭代瓶颈。
- 决策：在 `apps/desktop/src-tauri/Cargo.toml` 中为 dev profile 设置 `debug = 1`，减少调试信息生成量，以降低链接开销并加快增量迭代。
- 取舍：调试符号更少，但仍保留基本可用的回溯与调试能力；不影响 release 构建与运行时行为。
- 复核方式：
  - 在 Windows repo `apps/desktop/src-tauri/` 目录下，对比改动前后 `Measure-Command { cargo build }` 的耗时（尤其是增量编译 + 链接）。

## 新增 Windows 编译闸门（fail-fast）

VERIFIED（2026-02-10）

- 背景：WSL/Linux 上的 `cargo test` 不会编译 `#[cfg(windows)]` 的代码，导致 Windows-only 编译错误（例如 Tauri `manage()` 的 `Send+Sync` 约束）无法被现有 gate 提前发现。
- 决策：新增 `scripts/windows/windows_compile_gate.ps1`，要求在仓库根目录运行，执行 `apps/desktop/src-tauri` 的 `cargo check --locked` 作为快速闸门；并在 `scripts/windows/windows_gate.ps1` 开始阶段先运行它以 fail-fast。

## Windows Gate 可选启用 sccache（Rust 编译缓存）

VERIFIED（2026-02-09）

- 背景：Windows 侧仅用于编译/运行验证，Rust 依赖体量较大时，重复编译会明显拖慢迭代；同时不希望把额外工具作为硬依赖阻塞新环境启动（见 `docs/windows-gate-v0.1.md` 的“一键 gate”目标）。
- 决策：在 `scripts/windows/windows_gate.ps1` 中检测 `sccache`，若存在则自动设置 `RUSTC_WRAPPER=sccache` 并使用 repo-local `SCCACHE_DIR`；未安装时仅提示安装方式并继续执行。
- 取舍：将加速能力作为“可选增强”，避免因缺少 `sccache` 影响现有 Windows gate。
- 复核方式：
  - Windows 上执行 `scripts/windows/windows_gate.ps1`，应能看到 `sccache enabled` 或 `sccache not found (optional)` 的 INFO 输出；
  - 若启用后，运行 `sccache --show-stats` 观察 cache hits 增长。

## 更新默认“表达澄清”提示词模板为严格重写（转录文本场景）

VERIFIED（2026-02-10）

- 背景：转录文本可能包含错词/漏字/术语误转与大量口语碎片；旧的默认“表达澄清”system prompt 边界较宽，容易出现跑题、解释性输出、或对“优秀/更好”类词汇进行不必要细化。
- 决策：将内置默认模板中 `id="clarify"` 的 `system_prompt` 更新为“严格重写”规范：只做语义等价的书面化重写，禁止细化/省略/新增，并明确“指令免疫”和“只输出最终文本”。
- 方案：修改 `apps/desktop/src-tauri/src/templates.rs` 的 `default_templates()`（commit：`a6aa04a`）。
- 取舍：该变更只影响“未落盘 `templates.json` 的新环境/新用户”；若数据目录已有 `templates.json`（用户自定义模板），将继续按落盘内容优先，不自动覆盖。

## 结构化 Trace 日志：trace.jsonl（常开、落盘、可旋转）

VERIFIED（2026-02-10）

- 背景：用户明确要求“所有错误必须可定位到根因”，且 Windows release 无控制台，不能依赖 stdout/stderr。零散 event 日志无法覆盖跨线程任务、best-effort 分支与 early-return 错误路径。
- 决策：引入常开、结构化、可旋转的 `trace.jsonl`，并将其作为根因定位的第一诊断入口。
- 方案：`apps/desktop/src-tauri/src/trace.rs`
  - 输出：`TYPEVOICE_DATA_DIR/trace.jsonl`（超限自动旋转为 `trace.jsonl.1..N`）
  - 默认开启：`TYPEVOICE_TRACE_ENABLED=0` 可关闭
  - 默认记录 backtrace：`TYPEVOICE_TRACE_BACKTRACE=0` 可关闭
  - 失败写入：稳定 `step_id`、稳定错误码 `code`、error chain（`err_chain`）与运行时 backtrace（不手工维护 `file:line` 常量）
  - 脱敏：对 backtrace 中常见用户目录前缀做替换，避免泄漏个人绝对路径
- 取舍：日志是“始终存在的诊断成本”，但换取失败可定位与排障效率；同时通过 max-bytes+rotation 控制磁盘占用。

## Debug Payload 落盘：默认关闭，显式 Opt-In（含截图）

VERIFIED（2026-02-10）

- 背景：用户需要在调 prompt/定位问题时看到“LLM 实际收到的完整输入与完整输出”，但这类数据可能包含敏感信息（例如剪贴板内容、窗口截图）。
- 决策：将“完整 payload”落盘作为 Debug 能力，默认关闭，仅在显式环境变量开启时写入。
- 方案：`apps/desktop/src-tauri/src/debug_log.rs` + 各模块调用点（例如 `apps/desktop/src-tauri/src/llm.rs`、`apps/desktop/src-tauri/src/context_capture.rs`）
  - `TYPEVOICE_DEBUG_VERBOSE=1` 才会开启 debug payload 的写入
  - 细分开关（均为 opt-in）：
    - `TYPEVOICE_DEBUG_INCLUDE_LLM=1`：写 `debug/<task_id>/llm_request.json`、`llm_response.txt`
    - `TYPEVOICE_DEBUG_INCLUDE_ASR_SEGMENTS=1`：写 `debug/<task_id>/asr_segments.json`
    - `TYPEVOICE_DEBUG_INCLUDE_SCREENSHOT=1`：允许写 `debug/<task_id>/prev_window.png`
- 取舍：默认不落盘敏感数据，减少意外泄漏风险；需要时通过环境变量快速开启排障。

## Windows 上下文截图：双线性缩放 + 可配置最大边

VERIFIED（2026-02-10）

- 背景：默认缩放策略会导致截图过糊，影响上下文可读性与排障验证（用户需要“看一眼图片对不对”）。
- 决策：将缩放从最近邻调整为双线性插值，并提供最大边长配置以平衡清晰度与 token/带宽/延迟。
- 方案：`apps/desktop/src-tauri/src/context_capture_windows.rs`
  - 默认最大边：1600
  - 环境变量覆盖：`TYPEVOICE_CONTEXT_SCREENSHOT_MAX_SIDE`

## Prompt 实验脚本：打印完整输入输出，且不写入个人绝对路径元信息

VERIFIED（2026-02-10）

- 背景：提示词调参需要快速对比不同 system prompt 的效果，并且用户要求看到“完整输入输出”，同时脚本不得假设 WSL 或写入个人化路径。
- 决策：新增/完善 `scripts/llm_prompt_lab.py`，要求：
  - stdout 打印完整 request/response（便于直接粘贴对比）
  - 输出目录默认落在 repo 的 `tmp/` 下
  - 元信息展示避免泄漏个人绝对路径（仅显示 repo 相对路径或文件名）

## 全局快捷键输入：使用后端全局热键 + 前端 MediaRecorder + overlay 悬浮指示窗

VERIFIED（2026-02-10）

- 背景：用户反馈“每次都要切换到 UI 才能开始/结束录音”操作成本高，希望在任何地方都能快捷触发录音输入，且不能破坏现有功能（尤其是上一外部窗口截图链路）。
- 决策：
  - 后端使用 `tauri-plugin-global-shortcut` 注册系统级全局快捷键，并通过事件 `tv_hotkey_record` 通知前端；
  - 前端继续复用现有 `getUserMedia + MediaRecorder` 录音实现，避免引入新的平台录音依赖与大范围重构；
  - 增加一个小型 overlay window，用于在不切换主 UI 的情况下提示 `REC/TRANSCRIBING/COPIED/ERROR` 状态。
- 取舍：
  - 录音仍在前端，后端只负责全局热键与 pipeline，因此需要主应用处于运行状态（不做“未启动也能热键唤起/托盘常驻”）。
  - overlay 窗口的出现不会破坏“上一外部窗口截图”：选窗逻辑会排除本进程窗口（见 `apps/desktop/src-tauri/src/context_capture_windows.rs` 的 PID 过滤）。

## 修复类任务执行策略：原环境优先 + 最小闭环 + 禁止未授权扩展

VERIFIED（2026-02-11）

- 背景：在“修环境变量”任务中发生了任务边界漂移，擅自切换工作副本并触发额外安装，导致问题复杂化并偏离用户要求。
- 决策：
  - 对“修 bug / 修环境变量”类任务，默认只在用户原本已验证可用的环境操作。
  - 默认执行“最小闭环”：只改一项 -> 保证单进程 -> 日志复核。
  - 未经用户明确许可，禁止环境迁移、依赖安装、模型下载等范围外动作。
- 复核方式：
  - 执行记录中不出现新的工作副本路径与安装命令。
  - 每一步都能对应到单一变更项与可验证日志结果。

## WSL 触发 Windows 文档命令时的 PATH 同步策略（cargo 可见性）

VERIFIED（2026-02-11）

- 背景：用户要求“严格按文档命令执行”，但从 WSL 触发 Windows `npm run tauri dev` 时出现 `cargo metadata ... program not found`；根因是启动链路拿到旧 PATH。
- 决策：
  - 文档命令本体保持不变。
  - 若从 WSL 触发 Windows 命令，调用链必须提供“纯 Windows PATH + cargo bin”，禁止把 WSL 路径混入 Windows PATH（避免 `npm` 命中 `\\wsl.localhost\...`）。
- 复核方式：
  - `where cargo` 返回 `C:\Users\micro\.cargo\bin\cargo.exe`。
  - 执行文档命令后出现：
    - `Running DevCommand (cargo run ...)`
    - `Running target\\debug\\typevoice-desktop.exe`

## Windows Debug 的 FFmpeg 依赖以仓库 toolchain 为准（非 PATH）

VERIFIED（2026-02-11）

- 背景：
  - 用户反馈“Transcribe failed”，最新 trace 显示失败点为 `CMD.start_transcribe_recording_base64`，错误码 `E_TOOLCHAIN_NOT_READY`，提示仓库内 `toolchain/bin/windows-x86_64/ffmpeg.exe` 缺失。
  - `toolchain::selected_toolchain_dir()` 在 Windows Debug 模式优先选择仓库内 toolchain 目录；目录存在但二进制缺失时会在运行前校验阶段失败。
- 决策：
  - Windows Debug 环境的 FFmpeg 以仓库 `apps/desktop/src-tauri/toolchain/bin/windows-x86_64` 为准，不再将“系统 PATH 上可用 ffmpeg”视为充分条件。
  - 缺失时只使用仓库自带脚本恢复（与 manifest 对齐），不做版本不受控的旁路注入。
- 方案：
  - 在 Windows repo 执行：
    - `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\download_ffmpeg_toolchain.ps1 -Platform windows-x86_64`
  - 复核项：
    - `ffmpeg.exe/ffprobe.exe` 文件存在；
    - SHA256 与 `apps/desktop/src-tauri/toolchain/ffmpeg_manifest.json` 一致；
    - 重启后 `trace.jsonl` 出现 `TC.verify status=ok` 且 `expected_version=7.0.2`。

## 热键监听生命周期改为“单次订阅 + ref 读最新配置”（避免 rewrite 参数漂移）

VERIFIED（2026-02-11）

- 背景：
  - 用户反馈“UI 转录可 rewrite，但热键转录偶发不 rewrite”。
  - trace 证据显示在未更新 settings 的时间窗内，`CMD.start_transcribe_recording_base64` 的 `rewrite_enabled/template_id` 会在 `true/"correct"` 与 `false/null` 间漂移。
- 决策：
  - 前端事件监听采用“单次订阅”而非“随设置重绑”；
  - 所有会变化的运行时配置（`rewrite_enabled/template_id/hotkeys_enabled/show_overlay`）统一走 `ref` 读取最新值；
  - 对异步 `listen(...)` 注册添加取消保护，确保组件清理后不会残留旧监听。
- 方案：
  - 修改 `apps/desktop/src/screens/MainScreen.tsx`：
    - 新增配置/回调 `ref` 同步；
    - 监听 effect 依赖改为 `[]`，并使用 `trackUnlisten` + `cancelled` 防止泄漏；
    - `startRecording` 在启动时从 `ref` 快照 rewrite 参数。
- 取舍：
  - 增加少量 `ref` 与样板代码，换取监听生命周期确定性和热键路径参数一致性；
  - 避免后续再出现“同一 settings 下 UI 与热键行为不一致”。

## Windows 运行态与源码一致性的执行策略：先同步提交，再清理旧会话，最后单实例拉起

VERIFIED（2026-02-11）

- 背景：
  - 用户要求“把 Windows 上的进程更新到最新”。
  - 历史上多次出现“源码已更新但运行态仍是旧进程”或“多实例叠加导致热键冲突”。
- 决策：
  - 固化执行顺序为：
    1. 先让 Windows 工作副本 fast-forward 到主仓最新提交；
    2. 清理旧的 `node/cargo/typevoice-desktop` 相关会话；
    3. 再启动单一最新 `tauri dev` 会话并做连通性验证。
- 复核方式：
  - `git -C /mnt/d/Projects/TypeVoice rev-parse --short HEAD` 与主仓一致；
  - 进程列表只保留一套当前会话相关进程；
  - Windows 本机 `Invoke-WebRequest http://localhost:1420` 返回 `200`。
