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
- 本阶段新增聚焦目标（2026-02-11）：
  - 消除“UI 与热键路径 rewrite 行为不一致”，确保两条路径共享同一份实时配置语义（`rewrite_enabled/template_id`）。
  - 保持 Windows 运行态与当前源码一致（Windows 工作副本 `D:\\Projects\\TypeVoice` 与 WSL 主仓同 commit，且开发进程为最新会话单实例）。

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

- 热键 rewrite 一致性修复已在主仓落地：
  - 提交：`6af17a9`、`1df910c`
  - 文件：`apps/desktop/src/screens/MainScreen.tsx`
  - 要点：监听器单次订阅 + ref 读取最新配置 + 异步注册取消保护，避免旧闭包泄漏导致 `rewrite_enabled/template_id` 漂移。
- Windows 运行态已更新到最新：
  - WSL 主仓 `HEAD=1df910c`，Windows 工作副本 `D:\\Projects\\TypeVoice` 同步到 `1df910c`（fast-forward）。
  - 已清理旧会话并重启 `npm run tauri dev`；当前存在最新 `node/cargo/typevoice-desktop` 进程。
  - Windows 本机 `Invoke-WebRequest http://localhost:1420` 返回 `200`。
- 仍在持续关注的已知非阻断项：
  - `PrintWindow` 在部分窗口类型下仍可能产出全黑/窄图（best-effort，当前不修）。
  - `templates.json` 继续覆盖内置默认模板（排障时需优先检查 data dir 落盘模板）。

### Next

VERIFIED（零上下文新 agent 可直接继续的推进顺序）

1. 做热键 rewrite 回归闭环（优先级最高）：
   - 在当前 Windows 最新会话连续触发多次 PTT/Toggle。
   - 用 `trace.jsonl` 核对每次 `CMD.start_transcribe_recording_base64.ctx.rewrite_enabled/template_id` 是否稳定等于 settings。
   - 核对 `TASK.rewrite_effective` 是否进入 rewrite（`rewrite_entered=true`）。
2. 做完整链路回归（Linux + Windows）：
   - Windows：至少一轮 `Record -> Preprocess -> Transcribe -> Rewrite -> Persist -> Copy` 成功。
   - Linux：同步运行 `./.venv/bin/python scripts/verify_quick.py`，防止只测单侧。
3. 收敛未确认项：
   - 验证全新 Windows 终端会话是否无需 PATH 注入也能执行文档命令。
   - 视结果决定是否需要进一步固化“WSL 调用 Windows 命令”的标准封装。

## 当前工作集（关键文件 / 命令 / 约束）

### 关键文件

VERIFIED

- 结构化 trace：`apps/desktop/src-tauri/src/trace.rs`
- ContextCapture（聚合与 trace）：`apps/desktop/src-tauri/src/context_capture.rs`
- Windows 采集实现（截图/前台跟踪/剪贴板诊断）：`apps/desktop/src-tauri/src/context_capture_windows.rs`
- ContextPack 结构：`apps/desktop/src-tauri/src/context_pack.rs`
- LLM 调用与请求形态（text vs parts + image_url）：`apps/desktop/src-tauri/src/llm.rs`
- 热键/主流程前端入口（本轮重点）：`apps/desktop/src/screens/MainScreen.tsx`
- 模板默认值与 `templates.json` 覆盖：`apps/desktop/src-tauri/src/templates.rs`
- prompt 实验脚本：`scripts/llm_prompt_lab.py`，文档：`docs/llm-prompt-lab-v0.1.md`
- Gate：`scripts/verify_quick.py`、`scripts/verify_full.py`、`scripts/windows/windows_compile_gate.ps1`、`scripts/windows/windows_gate.ps1`
- AGENTS 索引更新脚本（技能）：`/home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py`

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
- 校验热键 rewrite 是否稳定（Windows data dir）：
  - `jq -c 'select(.step_id==\"CMD.start_transcribe_recording_base64\" and .status==\"ok\") | {ts_ms,ctx:.ctx}' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl | tail -n 30`
  - `jq -c 'select(.step_id==\"TASK.rewrite_effective\") | {ts_ms,task_id,ctx:.ctx}' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl | tail -n 30`
- 更新并校验 AGENTS 索引：
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py`
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --check`

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
- WSL 发起 PowerShell 命令时，若 `$` 未正确转义，可能被当前 shell 先行展开并污染脚本（例如把 `$_` 破坏成无效 token）；会导致排障命令产生误导性报错。见 `docs/memory/PITFALLS.md` 新增条目。

## 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- Windows 机器上的真实运行是否已开启 `TYPEVOICE_TRACE_ENABLED`（默认开启，但可能被环境覆盖）。
  - 推荐验证：在目标机器 data dir 查看 `trace.jsonl` 是否产生新行；若没有，检查环境变量。
- 选窗策略是否需要排除 Shell/任务栏窗口以降低“全黑/很窄截图”的概率。
  - 推荐验证：用 trace 对比 `CTX.prev_window.info` 记录的 `process_image` 与 `CTX.prev_window.screenshot` 的成功/尺寸分布，人工评估是否需要过滤。
- 是否需要在用户的“全局登录会话”层面完成 PATH 刷新（避免每次从 WSL 触发都要注入 PATH）。
  - 推荐验证：用户在全新 Windows Terminal 会话执行 `where cargo` 与文档命令，确认无需额外注入即可稳定拉起。
- 热键路径下“settings 显示 rewrite=true，但启动参数传入 false/null”在代码修复后是否完全消失。
  - 已实现修复（VERIFIED，代码 + 构建）：`apps/desktop/src/screens/MainScreen.tsx` 改为“监听单次注册 + 动态配置 ref 读取 + 异步注册取消保护”。
  - 待验证（UNCONFIRMED，运行时）：在 Windows Debug 单实例下连续触发热键录音，复核 `CMD.start_transcribe_recording_base64.ctx.rewrite_enabled/template_id` 是否稳定与 settings 一致。
- 当前正在运行的 Windows 最新会话在长时间运行下是否会再次出现多实例叠加（热键冲突）：
  - 推荐验证：每次重启前后都执行一次进程盘点，确保仅保留一套 `node/cargo/typevoice-desktop` 与一个有效的 dev 会话。

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

## 更正与最新状态（2026-02-11，追加）

VERIFIED

- 本轮用户反馈的“Transcribe failed”最新一次并非 `Transcribe` 阶段失败，而是命令入口拦截：
  - `trace.jsonl`：`CMD.start_transcribe_recording_base64 op=end status=err code=E_TOOLCHAIN_NOT_READY`
  - 错误消息：`missing ffmpeg binary at D:\\Projects\\TypeVoice\\apps\\desktop\\src-tauri\\toolchain\\bin\\windows-x86_64\\ffmpeg.exe`
- 直接根因：
  - `toolchain::selected_toolchain_dir()` 在 Windows Debug 模式优先选择仓库内 `apps/desktop/src-tauri/toolchain/bin/windows-x86_64`；
  - 该目录存在但二进制缺失（仅 `.gitkeep`），导致 `TC.verify` 失败并阻断任务启动。
- 已完成修复：
  - 在 Windows repo 执行 `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\download_ffmpeg_toolchain.ps1 -Platform windows-x86_64` 恢复工具链；
  - 复核 `ffmpeg.exe/ffprobe.exe` SHA256 与 `ffmpeg_manifest.json` 一致；
  - 重启 `tauri dev` 后 `trace.jsonl` 出现 `TC.verify status=ok`（`expected_version=7.0.2`）。

UNCONFIRMED

- 本轮重启后尚未再次手动触发录音任务来复核完整 `Preprocess -> Transcribe` 链路（环境预检已恢复为 `ok`）。

## 更正与最新状态（2026-02-11，hotkey rewrite 一致性）

VERIFIED

- 已完成根因澄清：并非后端存在“第二套 rewrite 设置读取源”；后端命令入口仅使用前端传入参数（见 `apps/desktop/src-tauri/src/lib.rs` 的 `start_transcribe_recording_base64`）。
- 已完成代码修复：
  - 文件：`apps/desktop/src/screens/MainScreen.tsx`
  - 变更点：
    - 热键/任务监听改为单次注册，避免依赖变更重绑造成闭包竞争；
    - `rewrite_enabled/template_id/hotkeys_enabled/show_overlay` 统一经 `ref` 读取最新值；
    - 对异步 `listen(...)` 注册增加取消保护，防止组件清理后旧监听残留。
- 本地验证：`apps/desktop` 下执行 `npm run build` 通过（`tsc` + `vite build`）。

UNCONFIRMED

- 尚未在本轮代码上完成 Windows 实机复测（连续热键录音 + trace 对齐）来最终关单该问题。

## 更正与最新状态（2026-02-11，Windows 进程已对齐最新）

VERIFIED

- 已按 `agents-md-project-index` 技能更新并校验 `AGENTS.md` 索引：
  - 更新命令：`python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --exclude-dir fixtures --exclude-dir metrics --exclude-dir models --exclude-dir target`
  - 校验命令：同脚本 `--check`（带相同 `--exclude-dir` 参数）返回通过。
- 已按用户要求将 Windows 运行进程更新到最新源码：
  - 主仓 `HEAD=1df910c`；
  - Windows 工作副本 `D:\\Projects\\TypeVoice` 已 fast-forward 到 `1df910c`。
- 已完成会话清理与重启：
  - 清理旧的 TypeVoice 开发进程；
  - 重新拉起 Windows 文档命令（`npm run tauri dev`）；
  - 进程侧可见新的 `node/cargo/typevoice-desktop`（`typevoice-desktop.exe` 路径为 `D:\\Projects\\TypeVoice\\apps\\desktop\\src-tauri\\target\\debug\\typevoice-desktop.exe`）。
- 连通性复核：
  - Windows 本机 `Invoke-WebRequest http://localhost:1420` 返回 `200`。

UNCONFIRMED

- 本次“更新到最新进程”完成后，尚未追加热键 rewrite 专项回归数据（trace 连续样本）；该验证仍需执行以关闭该问题。
