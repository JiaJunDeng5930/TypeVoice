# CONTINUITY（外置记忆）

说明：

- 本文件记录“当前进度与工作集”，用于跨回合恢复上下文。
- 内容必须区分 `VERIFIED`（能用代码/文档/命令复核）与 `UNCONFIRMED`（推断/未验证）。
- 冻结规格的真源在 `docs/*.md`；本文件只维护“可执行接手摘要”，避免复述导致漂移（真源摘要：`docs/memory/SPEC.md`）。

## 当前有效目标（与 SPEC 对齐）

VERIFIED（截至 2026-02-11）

- 产品目标：Windows 桌面端“录完再出稿”语音打字工具（录音结束 -> 本地 ASR -> 可选 LLM Rewrite -> 复制文本），MVP 优先可用性/稳定性/速度/可取消/可观测。真源：`docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`、`docs/verification-v0.1.md`。
- 本阶段目标：把“上下文 + 提示词 + 可诊断日志”固化为可复用流程，能快速迭代 prompt 并定位根因：
  - LLM 输入必须清晰区分 `### TRANSCRIPT` 与 `### CONTEXT`，避免模型把上下文当作待改写文本。
  - 对所有失败路径都能从落盘日志定位到根因（error chain + backtrace + step_id/code），不依赖控制台输出。

## 当前状态（Done / Now / Next）

### Done

VERIFIED（main 已包含，且可在本机复核）

- 结构化 trace 日志（常开、落盘、可旋转）：`TYPEVOICE_DATA_DIR/trace.jsonl`。
  - 覆盖命令入口、ContextCapture、Templates、LLM 调用等关键边界；失败会记录 `err_chain` 与 `backtrace`；并对常见用户路径做脱敏。实现：`apps/desktop/src-tauri/src/trace.rs`。
  - 开关与配置：
    - `TYPEVOICE_TRACE_ENABLED=0` 可关闭（默认开启）
    - `TYPEVOICE_TRACE_BACKTRACE=0` 可关闭 backtrace（默认开启）
    - `TYPEVOICE_TRACE_MAX_BYTES` / `TYPEVOICE_TRACE_MAX_FILES` 控制旋转（默认 10MB * 5）
- Debug verbose 工具链（用于“看完整输入输出”，默认关闭以保护隐私）：
  - `TYPEVOICE_DEBUG_VERBOSE=1` 后，LLM/ASR 等模块会落盘 debug payload 到 `TYPEVOICE_DATA_DIR/debug/<task_id>/...`（见 `apps/desktop/src-tauri/src/debug_log.rs`）。
  - 进一步细分开关：
    - `TYPEVOICE_DEBUG_INCLUDE_LLM=1`：写 `debug/<task_id>/llm_request.json` + `llm_response.txt`
    - `TYPEVOICE_DEBUG_INCLUDE_ASR_SEGMENTS=1`：写 `debug/<task_id>/asr_segments.json`
    - `TYPEVOICE_DEBUG_INCLUDE_SCREENSHOT=1`：允许落盘上一窗口截图 `debug/<task_id>/prev_window.png`（显式 opt-in）
- Windows ContextCapture：自动采集并附带到 Rewrite：
  - `RECENT HISTORY`（历史文本）、`CLIPBOARD`（剪贴板文本）、`PREVIOUS WINDOW`（TypeVoice 之前的外部前台窗口信息 + 截图，best-effort）。
  - 截图链路带诊断：失败会在 `trace.jsonl` 记录 step、WinAPI 名称、返回值与 `GetLastError`。见 `apps/desktop/src-tauri/src/context_capture.rs`、`apps/desktop/src-tauri/src/context_capture_windows.rs`。
- 截图质量：缩放采用双线性插值；最大边默认 1600，可用 `TYPEVOICE_CONTEXT_SCREENSHOT_MAX_SIDE` 覆盖。

- 全局快捷键录音输入 + overlay 悬浮指示（按住说话/按一下切换）：
  - 后端：`tauri-plugin-global-shortcut` 注册全局热键，触发时 emit `tv_hotkey_record`；
  - 前端：监听事件并复用现有 `MediaRecorder` 录音实现；
  - overlay：额外创建 `overlay` 窗口，前端通过 `overlay_set_state` 命令显示 `REC/TRANSCRIBING/COPIED/ERROR` 等状态。
  - 配置项写入 `settings.json`：`hotkeys_enabled`、`hotkey_ptt`、`hotkey_toggle`、`hotkeys_show_overlay`。
- Prompt 调试脚本（用于手动判断输出好坏，不做自动评测）：`scripts/llm_prompt_lab.py`。
  - 会打印完整 request/response 到 stdout，并落盘 `request.json`、`response_raw.json` 等到 `tmp/llm_prompt_lab/...`（不写入个人绝对路径元信息）。
- 质量闸门补强（避免“测试没覆盖到编译”）：
  - repo root 的 `scripts/verify_quick.py` / `scripts/verify_full.py` 已加入 `cargo check --locked`（后端编译闸门）。
  - Windows-only 快速编译闸门脚本：`scripts/windows/windows_compile_gate.ps1`（在 Windows PowerShell 下从 repo root 运行）。
- Windows release 启动健壮性：为 MSVC 目标设置更大主线程栈：`apps/desktop/src-tauri/.cargo/config.toml`（`/STACK:8388608`）。

### Now

VERIFIED（截至 2026-02-11）

- 已确认“上一窗口截图可能全黑/很窄”的现象：属于 `PrintWindow` 对特定窗口（例如 Shell/任务栏/硬件加速/受保护 surface）返回空像素的已知兼容性问题；当前用户决定先忽略不修。详见 `docs/memory/PITFALLS.md`。
- `templates.json` 覆盖内置默认模板：若 data dir 存在 `templates.json`，应用会优先使用落盘模板，而不是 `default_templates()`。实现：`apps/desktop/src-tauri/src/templates.rs`。
- 已完成“FFmpeg 路径回退”代码级复盘：
  - 最近 hotkeys/rewrite 提交未修改 FFmpeg 调用链；
  - FFmpeg 解析逻辑最近一次实质修改是 `d3de362`（引入 `resolve_tool_path`，优先 env/同目录，再 fallback PATH）；
  - 当前仓库 `tauri.conf.json` 未配置 `externalBin`，Windows Debug 目录也无 `ffmpeg.exe/ffprobe.exe`，因此 `tauri dev` 下在未配置 env 且 PATH 缺失时会报 `E_FFMPEG_NOT_FOUND`（属于环境/产物问题，不是 hotkey 改动回归）。
- 已确认 Windows 侧“此前可用、现在不可用”的直接原因：
  - `ffmpeg/ffprobe` 仍安装在 WinGet 目录（`%LOCALAPPDATA%\\Microsoft\\WinGet\\...`）；
  - 但当前 User/Machine PATH 均缺失 `%LOCALAPPDATA%\\Microsoft\\WinGet\\Links`，导致 `where ffmpeg` 失败；
  - 临时将该目录加到进程 PATH 后可立即解析，证明问题是 PATH 丢失而非二进制缺失。
- 已完成修复：
  - 将 `%LOCALAPPDATA%\\Microsoft\\WinGet\\Links` 持久写入用户 PATH，并规范化路径分隔（移除误写入的双反斜杠片段）。
  - 复核：在 Windows 合成环境（Machine+User PATH）下 `where ffmpeg/ffprobe` 与 `ffmpeg -version/ffprobe -version` 均通过；Linux 侧 `which ffmpeg/ffprobe` 也保持正常。
- 已确认并处理“热键不触发”根因：
  - 根因是 Windows Debug 多实例并存导致 `HotKey already registered`（`HK.register.ptt/toggle` 报错）。
  - 清理旧会话后，最新 `HK.apply` 为成功且无 `HK.register.* err`，热键注册恢复。
- 已确认并修复“文档命令下 `cargo metadata ... program not found`”：
  - 根因不是 `tauri dev` 命令本身，而是 WSL -> Windows 启动链路继承旧 PATH，导致 `cargo` 不可见。
  - `HKCU\Environment\Path` 已包含 `C:\Users\micro\.cargo\bin`，但当前调用链进程仍可能读取旧快照。
  - 已验证可行修复：注入“纯 Windows PATH + cargo bin”后，再执行同一条文档命令，成功进入 `Running DevCommand (cargo run ...)` 并启动 `target\\debug\\typevoice-desktop.exe`。

### Next

VERIFIED（零上下文新 agent 可直接继续的推进顺序）

1. 稳定 Windows Debug 拉起路径（优先级最高）：
   - 严格执行文档命令本体，不改写命令。
   - 若从 WSL 触发，先注入纯 Windows PATH（含 `C:\Users\micro\.cargo\bin`）再执行文档命令，避免旧 PATH 快照。
2. 继续业务回归验证（Linux + Windows 同步）：
   - 在 Windows Debug 单会话下验证录音 -> Preprocess -> ASR -> Rewrite -> 历史记录完整链路。
   - 同步跑 Linux 侧快速回归，避免“只测一边”。
3. 如需继续追根因（可选）：
   - 对 ContextCapture 的“上一窗口选择”策略做更精细过滤（例如排除 Shell/任务栏窗口），并用 trace 对比选窗与截图成功率。

## 当前工作集（关键文件 / 命令 / 约束）

### 关键文件

VERIFIED

- 结构化 trace：`apps/desktop/src-tauri/src/trace.rs`
- ContextCapture（聚合与 trace）：`apps/desktop/src-tauri/src/context_capture.rs`
- Windows 采集实现（截图/前台跟踪/剪贴板诊断）：`apps/desktop/src-tauri/src/context_capture_windows.rs`
- ContextPack 结构：`apps/desktop/src-tauri/src/context_pack.rs`
- LLM 调用与请求形态（text vs parts + image_url）：`apps/desktop/src-tauri/src/llm.rs`
- 模板默认值与 `templates.json` 覆盖：`apps/desktop/src-tauri/src/templates.rs`
- prompt 实验脚本：`scripts/llm_prompt_lab.py`，文档：`docs/llm-prompt-lab-v0.1.md`
- Gate：`scripts/verify_quick.py`、`scripts/verify_full.py`、`scripts/windows/windows_compile_gate.ps1`、`scripts/windows/windows_gate.ps1`

### 关键命令（不假设工作目录与磁盘路径）

VERIFIED

- repo root：`./.venv/bin/python scripts/verify_quick.py`
- repo root：`./.venv/bin/python scripts/verify_full.py`
- Windows 文档命令（在 Windows PowerShell 直接执行）：
  - `Set-Location D:\Projects\TypeVoice\apps\desktop`
  - `$env:RUST_BACKTRACE="1"; $env:RUST_LOG="debug"; npm run tauri dev`
- 从 WSL 触发 Windows 文档命令（仅修复调用链环境，不改命令本体）：
  - `WINPATH=$(/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "\$env:Path" | tr -d '\r')`
  - `PATH="C:\\Users\\micro\\.cargo\\bin;$WINPATH" WSLENV=PATH/w /mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -ExecutionPolicy Bypass -Command "Set-Location D:\Projects\TypeVoice\apps\desktop; \$env:RUST_BACKTRACE='1'; \$env:RUST_LOG='debug'; npm run tauri dev"`
- prompt lab（示例，参数按环境填写）：
  - `python scripts/llm_prompt_lab.py --base-url "$TYPEVOICE_LLM_BASE_URL" --model "$TYPEVOICE_LLM_MODEL" --api-key "$TYPEVOICE_LLM_API_KEY" --system-prompt-file tmp/prompt.txt --transcript "..." --clipboard "..." --prev-process "C:\\\\Windows\\\\explorer.exe"`
- 从 trace 中定位某次任务的错误：
  - `jq -r 'select(.status==\"err\") | {ts_ms,task_id,stage,step_id,error,ctx} | @json' \"$TYPEVOICE_DATA_DIR/trace.jsonl\" | tail`

### 关键约束

VERIFIED

- 不增加 UI 预览/勾选流程；上下文采集与发送应自动完成（用户偏好：`docs/memory/USER_PREFS.md`）。
- 诊断信息必须落盘可定位（见 `docs/memory/SPEC.md` 的“可诊断性”硬约束），且不得落盘 API key 与截图像素/base64（除非显式 debug 开关允许）。
- 禁止手工维护 `file:line` 常量；定位依赖运行时 backtrace + 稳定 `step_id`/`code`。
- 对 bug 修复/功能更新后的验证流程，必须包含 Windows 侧“同步最新源码 -> 重新编译 -> Debug 模式拉起”，并与 Linux 侧验证一起执行（不可只测单侧）。
- Windows 调试会话治理：同一时刻只保留一个最新有效 `tauri dev` 会话；拉起新会话前先清理旧会话，避免 `1420` 端口冲突与“多个 typevoice-desktop 进程”干扰排障。
- 用户明确要求“严格按文档命令执行”时：禁止改写命令本体、禁止额外包裹脚本；如失败先修环境再执行同一条文档命令。

## 风险与坑（指向 PITFALLS）

VERIFIED

- Windows release 无控制台：不要依赖 stdout/stderr；排障走 `trace.jsonl` 与 `debug/` 落盘。见 `docs/memory/PITFALLS.md`。
- `templates.json` 覆盖内置默认模板：提示词“看起来没生效”时优先检查 data dir。见 `docs/memory/PITFALLS.md`。
- 上一窗口截图可能全黑/空白：属于 WinAPI 兼容性与选窗策略交互，当前先忽略但会反复出现。见 `docs/memory/PITFALLS.md`。
- WSL 触发 Windows 命令时可能继承旧 PATH，导致 `cargo` 缺失或工具解析异常。见 `docs/memory/PITFALLS.md` 的“WSL 启动 Windows `tauri dev` 时继承旧 PATH”。

## 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- Windows 机器上的真实运行是否已开启 `TYPEVOICE_TRACE_ENABLED`（默认开启，但可能被环境覆盖）。
  - 推荐验证：在目标机器 data dir 查看 `trace.jsonl` 是否产生新行；若没有，检查环境变量。
- 选窗策略是否需要排除 Shell/任务栏窗口以降低“全黑/很窄截图”的概率。
  - 推荐验证：用 trace 对比 `CTX.prev_window.info` 记录的 `process_image` 与 `CTX.prev_window.screenshot` 的成功/尺寸分布，人工评估是否需要过滤。
- 是否需要在用户的“全局登录会话”层面完成 PATH 刷新（避免每次从 WSL 触发都要注入 PATH）。
  - 推荐验证：用户在全新 Windows Terminal 会话执行 `where cargo` 与文档命令，确认无需额外注入即可稳定拉起。

## 更正与最新状态（2026-02-11）

VERIFIED

- 本轮出现“问题越修越乱”的直接原因不是 FFmpeg/CUDA 本身失效，而是执行过程偏离了用户指定边界：
  - 用户要求是“就地修环境变量”，执行时却切换到了新的 Windows 工作副本 `C:\\Users\\micro\\Projects\\TypeVoice-win` 并启动了额外安装流程，导致引入了与原环境不同的新变量（如 `.venv`/模型状态不一致）。
- 已完成清理（按用户要求）：
  - 删除 `C:\\Users\\micro\\Projects\\TypeVoice-win\\.venv`
  - 删除 `C:\\Users\\micro\\Projects\\TypeVoice-win\\apps\\desktop\\src-tauri\\toolchain\\bin\\windows-x86_64\\ffmpeg.exe`
  - 删除 `C:\\Users\\micro\\Projects\\TypeVoice-win\\apps\\desktop\\src-tauri\\toolchain\\bin\\windows-x86_64\\ffprobe.exe`
  - 删除 `C:\\Users\\micro\\Projects\\TypeVoice-win\\tmp\\windows-debug`
  - 复核结果：上述路径均不存在，且 `NO_TYPEVOICE_PROCS`（无 TypeVoice 相关残留进程）。
- 后续执行约束已落盘到 `USER_PREFS.md` 与 `DECISIONS.md`：
  - 修复类任务必须“原环境 + 最小闭环 + 单变量变更”，禁止未获授权的环境迁移和额外安装。

## 更正与最新状态（2026-02-11，补充）

VERIFIED

- 本轮已完成 `AGENTS.md` 索引更新（使用 `agents-md-project-index` 技能脚本），并通过 `--check` 校验。
- 本轮确认并修复了 Windows Debug 拉起失败的直接根因：
  - 错误表现：`failed to run 'cargo metadata' ... program not found`
  - 根因：从 WSL 调用 Windows 命令时继承旧 PATH，`cargo` 不可见。
  - 修复结果：在调用链注入“纯 Windows PATH + cargo bin”后，文档命令成功拉起（出现 `Running DevCommand (cargo run ...)` 与 `Running target\\debug\\typevoice-desktop.exe`，并观测到 `typevoice-desktop` 进程存活）。
