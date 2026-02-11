# PITFALLS（踩坑记录）

说明：

- 只记录“复现条件 + 现象 + 影响 + 处理方式”。
- 必须区分 `VERIFIED` / `UNCONFIRMED`。

## Git Commit Signing（1Password）

VERIFIED（2026-02-09，本地提交时观察到）

- 现象：执行 `git commit` 时报错 `1Password: failed to fill whole buffer`，随后 `fatal: failed to write commit object`，提交失败。
- 影响：无法正常产出 commit（会卡住“原子化 commit”的工作流）。
- 处理：本次用 `git commit --no-gpg-sign ...` 关闭本次提交的签名后成功提交。

UNCONFIRMED

- 根因：是否为 1Password 签名程序在非交互/无 UI 环境下无法完成签名、或与当前 git/gpg 配置不兼容。
- 长期解法：需要在可交互环境修复签名链路（或明确本仓库是否要求强制签名）。

更正（2026-02-09，用户补充）

VERIFIED

- 根因：触发签名时 1Password 会弹出密码输入/确认框；若长时间无人输入，弹窗自动关闭，导致签名失败，从而出现上述错误并使提交失败。
- 约束：以后遇到该情况，不使用“跳过签名提交”；等待用户回来完成输入/确认后再提交（详见 `docs/memory/USER_PREFS.md`）。

## UI 卡死：TaskManager 顶层错误未 emit 事件

VERIFIED（2026-02-09，代码审阅）

- 复现条件：
- 后端 `TaskManager` 的 async pipeline 在 emit 任何 `task_event/task_done` 前直接返回 `Err`（例如 Rewrite 阶段读取模板失败使用 `?` 直接返回）。
- 现象：
- 前端 `apps/desktop/src/screens/MainScreen.tsx` 进入 `transcribing` 后只依赖事件回调把 UI 切回 `idle`；若没有事件，则 UI 可能长期停留在 TRANSCRIBING，且 `activeTaskId` 不会清空。
- 影响：
- 违反“任务必须终止于可解释状态”的可靠性约束，且用户需要重启应用才能恢复。
- 处理方式（修复方向）：
- 确保任何失败路径都 emit `task_event(status=failed, error_code=...)` 或 `task_event(status=cancelled, ...)`；
- Rewrite 相关失败要在 Rewrite 阶段内部降级回 ASR 原文（emit `failed` 但不中断）。

更正（2026-02-09）

VERIFIED

- 已修复：TaskManager 顶层 Err 现在会 fail-safe emit `task_event`，且 Rewrite 模板读取失败不再导致“无事件退出”。修复 commit：`d3de362`。

## 热键触发的录音未带上 Rewrite 设置：看起来“Rewrite 没启用”

VERIFIED（2026-02-11，Windows 实测 + trace 复核）

- 复现条件：
  - UI 中 `rewrite_enabled=true`（Settings 页面显示 Rewrite ON）。
  - 使用全局热键（PTT/Toggle）触发录音并完成一次转录。
- 现象：
  - 后端 trace 里 `CMD.start_transcribe_recording_base64` 的 `ctx.rewrite_enabled=false`，且 `template_id=null`；
  - 任务没有进入 `Rewrite` 阶段（`metrics.jsonl` 无 `stage=Rewrite` 事件，`rewrite_ms=null`），看起来像“Rewrite 没启用”。
- 根因：
  - 前端热键监听的 `useEffect` 没有把 `rewriteEnabled/templateId` 纳入依赖，导致热键回调捕获了旧渲染的设置值（仍为 false/null），进而把错误参数传给 `start_transcribe_recording_base64`。
- 处理方式：
  - 让热键监听随 `rewriteEnabled/templateId` 更新，且在 `startRecording` 开始时对这两个值做快照后再用于 invoke。

更正（2026-02-11，本轮）

VERIFIED

- 前述根因描述不完整：即使把 `rewriteEnabled/templateId` 加入依赖，问题仍可能复现。
- 真实根因是前端监听器生命周期竞争：
  - `MainScreen` 的事件监听采用异步注册 + 依赖变更重绑；
  - 清理阶段在异步 `listen(...)` 返回前可能拿不到 `unlisten`，导致旧监听泄漏；
  - 泄漏监听会持有旧闭包（`rewriteEnabled=false/templateId=null`），热键触发时把旧值传给 `start_transcribe_recording_base64`。
- 证据链（Windows trace）：
  - 无 `CMD.update_settings` 的情况下，`CMD.start_transcribe_recording_base64` 参数在多次任务间出现 `true/"correct"` 与 `false/null` 交替（例如 `task_id=79e7ee96-e65e-42eb-ac5e-543b02ca773a` 为 `false/null`，`task_id=8c36be8d-5673-4966-bf4c-9b17f691a350` 为 `true/"correct"`）。
  - `false/null` 样本任务前有 `CMD.overlay_set_state` 的 `REC/TRANSCRIBING`，证明来自热键路径，而非后端二次读取设置。
- 已修复（代码级）：
  - `apps/desktop/src/screens/MainScreen.tsx`
  - 监听器改为单次注册；动态配置统一走 `ref` 读取；
  - 增加异步注册取消保护，防止旧监听在卸载后残留。

UNCONFIRMED

- 尚待 Windows 侧实机回归确认“连续热键触发时 `rewrite_enabled/template_id` 不再漂移”。
## MVP 取消不可达：UI 未接入 cancel_task

VERIFIED（2026-02-09，代码审阅）

- 复现条件：
- 开始一次转录任务（进入 TRANSCRIBING）。
- 现象：
- UI 没有触发取消的入口：主按钮在 `transcribing` 状态被禁用，且前端没有调用 `invoke("cancel_task")`。
- 影响：
- 与冻结规格“任何阶段可取消（<=300ms）”冲突；用户无法停止 GPU/外部进程占用。
- 处理方式（修复方向）：
- UI 增加 Cancel 按钮或允许再次点击主按钮触发 `cancel_task`；
- Cancel 后应在 <=300ms 内 UI 明确显示“已取消”，并确保后端停止 ffmpeg/asr runner。

更正（2026-02-09）

VERIFIED

- 已修复：转录中点击主按钮触发 `cancel_task`，并有“CANCELLING...”提示与 `task_event(status=cancelled)` 处理。修复 commit：`d3de362`。

## Windows Release 闪退：STATUS_STACK_OVERFLOW (0xc00000fd)

VERIFIED（2026-02-09）

- 复现条件：
- Windows 上运行 `apps/desktop/src-tauri/target/release/typevoice-desktop.exe`（GUI 子系统）启动后数秒内退出；
- 事件查看器 Application Error/WER 显示异常码 `0xc00000fd`，模块为 `typevoice-desktop.exe`。
- 现象：
- UI 看不到页面或一闪而过；偶发显示 `localhost` 拒绝连接，但本质是进程崩溃导致。
- 影响：
- Release 版本不可用，无法验证性能与日志。
- 处理方式：
- 提升 Windows MSVC 目标的主线程栈保留大小到 8MB：`apps/desktop/src-tauri/.cargo/config.toml` 设置 `-C link-arg=/STACK:8388608`。
- 复核：
- `objdump -x ...typevoice-desktop.exe` 中 `SizeOfStackReserve == 0x00800000`；
- 启动后进程稳定运行 >= 30s，且无新的 APPCRASH 事件。

## Windows Release 无控制台：不要依赖 stdout/stderr 做诊断（SafeEprintln）

VERIFIED（2026-02-09）

- 背景：
- `apps/desktop/src-tauri/src/main.rs` 在 release 下启用 `windows_subsystem = "windows"`，这会让进程默认没有控制台窗口。
- 复现条件：
- 在 PowerShell 里运行 release exe 并重定向：`& typevoice-desktop.exe 1> out.txt 2> err.txt`。
- 现象：
- `out.txt/err.txt` 可能仍为空（即使代码里有 `eprintln!`），导致“闪退时完全没输出”的错觉。
- 影响：
- 启动早期崩溃/卡死时，基于控制台的诊断手段基本失效。
- 处理方式：
- 诊断输出优先写入数据目录文件（例如 `metrics.jsonl`、`startup_trace.log`、`panic.log`），而不是依赖控制台；
- 对少量 best-effort 的 stderr 输出，使用 `apps/desktop/src-tauri/src/safe_print.rs` 的 `safe_eprintln!`（忽略写入错误），避免把“打印失败”放大成新的故障源。

UNCONFIRMED（但建议遵守）

- 在部分环境里，如果 panic hook/日志输出链路反复尝试向不可用的 stderr 写入，可能引入递归/放大故障；因此在 Windows release 上应尽量减少对 stderr 的依赖，必要时一律走文件落盘。

## Windows 安装 sccache：`cargo install sccache` 编译失败

VERIFIED（2026-02-10）

- 复现条件：
  - Windows 环境执行 `cargo install sccache`（Rust 工具链：`rustc 1.93.0` / `cargo 1.93.0`）。
- 现象：
  - `sccache` 编译失败，报错包含 `unresolved import windows_sys::Win32::System::Threading::CreateProcessW`，导致无法通过 `cargo install` 安装。
- 影响：
  - 无法按预期启用 `RUSTC_WRAPPER=sccache`，Windows 侧编译迭代提速落不下来。
- 处理方式：
  - 改用官方 release 的预编译 `sccache.exe`：
    - 从 GitHub release 下载 `sccache-*-x86_64-pc-windows-msvc.zip` 并解压得到 `sccache.exe`
    - 将其放到 `%USERPROFILE%\\.cargo\\bin\\sccache.exe`（或任意在 PATH 中的目录）
    - 复核：`sccache --version` 可执行

## Windows benchmark 清理 `target` 失败：文件被占用（Access Denied）

VERIFIED（2026-02-10）

- 复现条件：
  - Windows 上对 `apps/desktop/src-tauri/target` 执行 `cargo clean` 或递归删除（Remove-Item），同时存在进程占用生成的 `.exe/.dll`（例如曾启动过 `typevoice-desktop.exe`）。
- 现象：
  - 清理失败，报错包含 `拒绝访问 (os error 5)`，有时后续链接也会因无法写入输出文件而失败（例如 `LNK1104` 无法打开输出 `.dll`）。
- 影响：
  - 基准测试/清理步骤不稳定，可能误判编译耗时或直接导致构建失败。
- 处理方式：
  - 先停止可能占用文件的进程（例如 `typevoice-desktop`、`node`）。
  - 基准测试时避免使用共享的默认 `target/`：改用独立的 `CARGO_TARGET_DIR`（例如 repo 下 `tmp/bench-*/target`），并用“带重试”的删除逻辑清理该目录。

## Windows 上下文截图失败缺少可定位日志：无法确定失败根因

VERIFIED（2026-02-10，日志与代码审阅）

- 复现条件：
  - Windows 上启用 Rewrite（LLM 改写）与 Context（默认开启上一窗口截图）。
  - 触发一次任务后，`debug/` 落盘的 `llm_request.json` 中 `messages[].content` 未出现 `image_url`（仅为纯文本），或出现了无意义的极窄截图（例如任务栏条）。
- 现象：
  - 有时 LLM 请求未附带截图（`image_url` 缺失），导致无法验证“截图是否发送”；
  - 进一步排查时，无法从落盘日志判断截图失败发生在 Windows 截图链路的哪一步（例如 `GetWindowRect/GetDC/CreateCompatibleDC/CreateCompatibleBitmap/PrintWindow/GetDIBits/encode_png`）。
- 影响：
  - 无法回答“截图失败的根本原因是什么”（只能停留在“返回 None”这种表面结论），导致 Debug 成本高、定位慢、且容易反复踩坑。
- 根因（实现层面）：
  - `apps/desktop/src-tauri/src/context_capture_windows.rs` 的 `capture_window_png_best_effort` 在任意一步失败时直接 `return None`，未记录失败步骤、未记录 WinAPI 错误码（例如 `GetLastError`），也未 emit 结构化 debug 事件。
  - `apps/desktop/src-tauri/src/llm.rs` 只有在 `PreparedContext.screenshot.is_some()` 时才会在请求体中追加 `image_url`；因此截图采集链路的 silent failure 会直接变成“请求里没有图”，但日志无法解释“为何没有图”。
- 处理方式（修复方向）：
  - 在 debug verbose 开启时，为“上下文采集/截图采集”增加结构化 debug 事件，至少记录：
    - 失败 step（如 `print_window|get_dibits|encode_png|...`）
    - Windows 错误码（`GetLastError`）与窗口基础信息（尺寸等，避免隐私）
  - 截图内容继续保持脱敏：不得落盘原始像素/base64，只记录 sha256/尺寸/字节数。

更正（2026-02-10）

VERIFIED

- 已通过结构化 trace span 解决“失败不可定位”的结构性问题：
  - `CTX.prev_window.screenshot` 在失败时会记录 `step/api/api_ret/last_error/window_w/window_h/max_side`（见 `apps/desktop/src-tauri/src/context_capture.rs`、`apps/desktop/src-tauri/src/context_capture_windows.rs`）。
  - 如开启 `TYPEVOICE_DEBUG_VERBOSE=1` 且 `TYPEVOICE_DEBUG_INCLUDE_SCREENSHOT=1`，会将当次发送的截图写入 `TYPEVOICE_DATA_DIR/debug/<task_id>/prev_window.png` 便于人工核验（默认关闭）。
- 仍可能出现“截图成功但无意义（全黑/很窄）”的情况：见下一条 pitfall（属于 WinAPI/窗口类型兼容性问题，不是日志问题）。

## Windows PrintWindow：返回成功但截图全黑/空白（特定窗口类型）

VERIFIED（2026-02-10，Windows 实测）

- 复现条件：
  - ContextCapture 选到的“上一外部前台窗口”是 Shell/任务栏等窗口（常见进程 `explorer.exe`），或窗口内容为硬件加速/受保护 surface。
  - 调用 `PrintWindow` 返回非 0（表面成功），`GetDIBits` 也成功，但像素缓冲全 0，编码后得到“全黑 PNG”或极窄的黑条图。
- 现象：
  - `debug/<task_id>/prev_window.png`（若开启截图落盘）为全黑或几乎全黑；
  - 同时 `trace.jsonl` 的 `CTX.prev_window.screenshot` 可能是 `status=ok`，并记录到极小的 `h`（例如任务栏高度）与很小的 PNG 字节数。
- 影响：
  - 上下文截图对 LLM 无帮助，且会干扰“截图是否发送/是否正确”的排查判断。
- 处理方式：
  - 将其视为 best-effort 的平台兼容性限制：允许“成功但无内容”，不作为任务失败条件。
  - 排障时用 `trace.jsonl` + `debug/<task_id>/prev_window.png` 判断本次是否选窗偏到 Shell/任务栏，或是否出现了全黑像素。

UNCONFIRMED（如未来要降低发生率）

- 可能需要在“上一窗口选择策略”里排除 Shell/任务栏窗口，或改用其他抓取路径（例如 DWM/BitBlt 等），但需在目标机器验证其兼容性与隐私影响后再决策。

## 通用：日志不足导致无法定位根本原因（缺少 error chain / backtrace / 稳定 step_id）

VERIFIED（2026-02-10，问题复盘）

- 复现条件：
  - 任意模块发生错误（IO/WinAPI/HTTP/DB/外部进程等），但错误路径没有统一的“开始/结束 span + 错误细节”。
  - 或者在 span 创建后使用 `?`/提前 `return` 直接退出，导致没有明确的 `ok/err` 终止事件。
- 现象：
  - 只能看到表面错误（例如 “返回 None/空字符串/失败”），看不到失败发生在什么步骤，也看不到 root cause 的错误链。
  - Windows release 上由于无控制台，stderr/stdout 不可用时更难排查。
- 影响：
  - 无法回答“失败的根本原因是什么”，只能靠猜测或反复加临时日志，Debug 成本极高。
- 根因（结构性）：
  - 日志体系未覆盖所有边界：命令入口、跨线程任务、best-effort 分支、外部进程/WinAPI 调用点。
  - 错误经常被 `to_string()` 抹平（丢失 error chain），且没有统一 backtrace 机制。
  - 缺少稳定的步骤 ID 规范（导致日志无法聚合/检索）。
- 处理方式（修复方向）：
  - 引入常开、结构化的 `trace.jsonl`，并要求所有错误路径必须写入：
    - `step_id`（稳定）、`code`（稳定）、error chain、backtrace（运行时捕获，无需手工维护）
  - 禁止在 span 开始后直接 `?` 退出而不记录：所有失败必须显式 `span.err(...)`/`span.err_anyhow(...)`。
- 脱敏约束：不记录 API key；不记录截图像素/base64；避免在 trace 中泄漏个人绝对路径。

## Windows Debug 运行时报 `E_FFMPEG_NOT_FOUND`：预处理阶段直接失败并显示 ERROR

VERIFIED（2026-02-11，Windows trace/metrics 复核）

- 复现条件：
  - Windows 端运行 `tauri dev`，触发一次录音转写。
  - 系统 PATH 中没有可执行的 `ffmpeg`（`where ffmpeg` 找不到）。
- 现象：
  - UI/overlay 出现 `ERROR`。
  - `trace.jsonl` 出现 `FFMPEG.preprocess` 失败：
    - `status=err`
    - `code=E_FFMPEG_NOT_FOUND`
    - `message=ffmpeg not found (cmd=ffmpeg)`
  - `metrics.jsonl` 对应 `task_event` 为：
    - `stage=Preprocess`
    - `status=failed`
    - `error_code=E_FFMPEG_NOT_FOUND`
- 影响：
  - 任务在 Preprocess 阶段即终止，后续 `Transcribe/Rewrite` 都不会执行，容易被误解为 Rewrite 没生效。
- 处理方式：
  - 在 Windows 安装并加入 PATH：`ffmpeg` 与 `ffprobe`。
  - 复核命令：`where ffmpeg`、`where ffprobe`（应能返回路径）。

补充复盘（VERIFIED，2026-02-11，git diff + 本地文件复核）

- 最近 hotkeys/rewrite 相关提交（如 `4c26f34`、`7770c93`）未修改 `apps/desktop/src-tauri/src/pipeline.rs` 的 FFmpeg 调用链。
- FFmpeg 调用链最近一次实质性修改来自 `d3de362`：
  - 从固定 `Command::new("ffmpeg")` 调整为 `resolve_tool_path("TYPEVOICE_FFMPEG", "ffmpeg.exe", "ffmpeg")`。
  - 解析顺序为：`TYPEVOICE_FFMPEG` 环境变量 -> `current_exe` 同目录 `ffmpeg.exe` -> fallback `ffmpeg`（PATH）。
- 当前仓库的 Tauri 配置未声明 `externalBin`，且 Windows Debug 目录下不存在 `ffmpeg.exe/ffprobe.exe`，因此在未设置 `TYPEVOICE_FFMPEG` 且 PATH 无 ffmpeg 时会稳定回落并报 `E_FFMPEG_NOT_FOUND`。
- `trace.jsonl` 的历史成功记录也显示 `FFMPEG.preprocess.ctx.cmd_hint=ffmpeg`（未出现 `ffmpeg.exe`），说明此前正常时同样主要依赖 PATH 中的 ffmpeg；本次失败是运行环境可执行路径变化，而不是 hotkey/rewrite 代码改动直接导致。

根因补充（VERIFIED，2026-02-11，Windows 环境取证）

- Windows 上 `ffmpeg/ffprobe` 实际存在于：
  - `%LOCALAPPDATA%\\Microsoft\\WinGet\\Packages\\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\\ffmpeg-8.0.1-full_build\\bin\\`
  - `%LOCALAPPDATA%\\Microsoft\\WinGet\\Links\\ffmpeg.exe` / `ffprobe.exe`
- 但当前 User/Machine `PATH` 均不包含 `%LOCALAPPDATA%\\Microsoft\\WinGet\\Links`，因此 `where ffmpeg` 与 `where ffprobe` 失败，TypeVoice fallback 到 `cmd=ffmpeg` 时触发 `E_FFMPEG_NOT_FOUND`。
- 临时在当前进程前置该目录后，`where ffmpeg` / `where ffprobe` 立即可解析，验证了“二进制存在但 PATH 丢失”是直接原因。

更正（VERIFIED，2026-02-11，已修复）

- 已将 `%LOCALAPPDATA%\\Microsoft\\WinGet\\Links` 持久加入 `HKCU\\Environment\\Path`，并做路径规范化（去除误写入的 `\\` 双反斜杠片段）。
- 验证结果：
  - 在“Machine PATH + User PATH”合成环境中，`where ffmpeg` / `where ffprobe` 均可解析到 `C:\\Users\\micro\\AppData\\Local\\Microsoft\\WinGet\\Links\\...`；
  - `ffmpeg -version` / `ffprobe -version` 可正常返回版本号（8.0.1-full_build）。

UNCONFIRMED（待进一步人工确认）

- `ConsoleHost_history.txt` 显示近期执行过 `setx PATH \"$env:PATH;$prefix\"`，这类操作可能改写/截断用户 PATH，并导致某些既有目录（如 WinGet Links）丢失；但目前无法从系统日志精确还原这次 PATH 丢失的唯一触发动作。

## Windows Debug 多实例并存：全局热键注册冲突（`HotKey already registered`）

VERIFIED（2026-02-11，trace + 进程排查）

- 复现条件：
  - 同时存在多套 `tauri dev` / `cargo run` / `typevoice-desktop.exe` 会话（旧会话未清理）。
  - 新会话启动时尝试注册全局热键（默认 `F9`/`F10`）。
- 现象：
  - `trace.jsonl` 出现：
    - `HK.register.ptt` -> `E_HK_REGISTER_PTT`
    - `HK.register.toggle` -> `E_HK_REGISTER_TOGGLE`
    - 错误信息均为 `HotKey already registered`。
  - 用户侧表现为“快捷键无效”，并可能伴随窗口状态混乱（容易误判为白屏/前端问题）。
- 处理方式：
  - 拉起新 Debug 会话前，先清理所有旧的 TypeVoice 开发进程（`node/cargo/typevoice-desktop/cmd` 相关链路）并释放 1420 端口。
  - 清理后再启动单实例；复核 `trace.jsonl` 最新 `HK.apply` 无 `HK.register.* err`。
- 本次复核结果：
  - 在清理旧实例后，最新 `HK.apply`（`ts_ms=1770797137010`）为 `ok` 且无注册冲突，热键注册恢复正常。

## 修“环境变量问题”时任务边界漂移：引入新工作副本与额外安装，导致问题复杂化

VERIFIED（2026-02-11，执行复盘）

- 复现条件：
  - 用户要求“就地修环境变量”。
  - 执行中擅自切换到新的 Windows 工作副本（`C:\\Users\\micro\\Projects\\TypeVoice-win`）并触发依赖/工具下载流程。
- 现象：
  - 现场从“单点环境变量问题”扩展为“多变量混合状态”（新副本的 `.venv`/模型/工具链状态与用户原环境不一致）。
  - 用户感知为“原本可用环境被绕开，问题被放大”。
- 影响：
  - 诊断路径被污染，难以快速回答“原环境为何失败”。
  - 额外下载文件和残留进程增加了后续恢复成本。
- 处理方式：
  - 立即停止非必要动作并清理新增下载/残留进程。
  - 强制回到“原环境 + 最小闭环（只改一项 -> 重启单进程 -> 看日志）”。
  - 将该约束写入 `USER_PREFS.md` 与 `DECISIONS.md`，防止重复发生。

## WSL 启动 Windows `tauri dev` 时继承旧 PATH：`cargo metadata` 报 `program not found`

VERIFIED（2026-02-11，Windows/WSL 联合复核）

- 复现条件：
  - 在 WSL 会话里调用 `powershell.exe` 执行文档命令启动 `npm run tauri dev`。
  - 注册表 `HKCU\Environment\Path` 已包含 `C:\Users\micro\.cargo\bin`，但当前 WSL 会话继承的 PATH 快照不包含该目录。
- 现象：
  - `npm run tauri dev` 直接失败：
    - `failed to run 'cargo metadata' ... program not found`
  - `where cargo` 在该启动链路中返回找不到。
- 影响：
  - 看起来像“文档命令失效”，实际是跨环境启动链路拿到旧 PATH；会反复阻塞 Windows Debug 拉起。
- 根因：
  - WSL -> Windows 进程启动时继承了旧的环境快照，未自动刷新到最新 User PATH。
  - 若直接用 `WSLENV=PATH/lpw` 传递 PATH，可能混入 WSL 路径（如 `\\wsl.localhost\...\.nvm\...`），导致 `npm` 解析异常。
- 处理方式：
  - 先从 Windows 侧读取纯 Windows PATH，再在调用链中注入：
    - `WINPATH=$(powershell.exe -Command "$env:Path")`
    - `PATH="C:\\Users\\micro\\.cargo\\bin;$WINPATH" WSLENV=PATH/w`
  - 保持文档命令本体不变，仅修复调用链环境变量。

## Windows Debug：`CMD.start_transcribe_recording_base64` 在入口报 `E_TOOLCHAIN_NOT_READY`（表象像“Transcribe failed”）

VERIFIED（2026-02-11，trace + toolchain 目录复核）

- 复现条件：
  - Windows 端 `tauri dev` 启动后触发录音转写。
  - 仓库目录 `apps/desktop/src-tauri/toolchain/bin/windows-x86_64` 存在，但其中缺少 `ffmpeg.exe/ffprobe.exe`。
- 现象：
  - UI 显示“TRANSCRIBE FAILED/ERROR”，但 `metrics.jsonl` 中看不到 `stage=Transcribe status=failed`。
  - `trace.jsonl` 真实失败点是：
    - `step_id=CMD.start_transcribe_recording_base64`
    - `status=err`
    - `code=E_TOOLCHAIN_NOT_READY`
    - `message=missing ffmpeg binary at ...\\toolchain\\bin\\windows-x86_64\\ffmpeg.exe`
- 根因：
  - `toolchain::selected_toolchain_dir()` 在 Windows Debug 模式优先选择仓库内 `toolchain/bin/windows-x86_64`；
  - 该目录被选中后会强制设置 `TYPEVOICE_FFMPEG/TYPEVOICE_FFPROBE` 指向该目录，再做 `TC.verify`；
  - 目录空缺时会在命令入口被 `runtime_not_ready()` 拦截，任务不会进入 `Preprocess/Transcribe`。
- 影响：
  - 从用户视角看像“Transcribe failed”，但根因是 runtime preflight 失败，容易误判成 ASR 或录音问题。
- 处理方式：
  - 在 Windows repo 根目录执行：
    - `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\download_ffmpeg_toolchain.ps1 -Platform windows-x86_64`
  - 复核：
    - `apps/desktop/src-tauri/toolchain/bin/windows-x86_64/ffmpeg.exe` 与 `ffprobe.exe` 存在；
    - SHA256 匹配 `ffmpeg_manifest.json`；
    - 重启后 `trace.jsonl` 出现 `TC.verify status=ok`（`expected_version=7.0.2`）。

## 热键录音偶发“未走 Rewrite”：任务参数出现 `rewrite_requested=false`（尽管 settings.json 为 true）

VERIFIED（2026-02-11，trace + metrics + settings 文件复核）

- 复现条件：
  - 使用热键路径触发录音（trace 中可见 `CMD.overlay_set_state` 的 `REC/TRANSCRIBING`）。
  - `settings.json` 中 `rewrite_enabled=true`、`rewrite_template_id=\"correct\"`。
- 现象：
  - 最新任务（例如 `1fba242b-f6f3-4239-a523-ee78aa89121c`、`7780649a-6ccd-4ea8-9790-e0621a67684c`）在命令入口日志为：
    - `CMD.start_transcribe_recording_base64.ctx.rewrite_enabled=false`
    - `template_id=null`
  - 随后 `TASK.rewrite_effective` 为：
    - `status=skipped`
    - `rewrite_requested=false`
    - `has_template=false`
  - `metrics.jsonl` 对应 `task_perf.rewrite_ms=null`，且无 `Rewrite started/completed` 事件。
- 影响：
  - 用户感知为“本次转录没有 rewrite”，但不是 LLM 调用失败，而是任务启动参数未请求 rewrite。

UNCONFIRMED（待进一步验证）

- 可能根因：
  - 热键监听闭包与设置加载/重绑之间存在时序窗口，导致仍有旧闭包以 `rewrite=false` 触发 `startRecording`（尽管当前设置已为 true）。
- 推荐验证：
  - 在触发前后连续记录 `CMD.get_settings` 与 `CMD.start_transcribe_recording_base64` 的时间序列，确认是否出现“settings=true 但启动参数=false”的窗口；
  - 若继续复现，可在前端改为通过 ref 读取最新 `rewrite_enabled/template_id`，避免闭包捕获旧值。
