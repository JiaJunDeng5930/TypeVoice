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

## 更正与最新状态（2026-02-11，热键 rewrite 回归结果）

VERIFIED

- 已在 Windows 最新会话完成连续样本复核，结论是问题仍存在：
  - `CMD.get_settings`（`ts_ms=1770818689915`）返回 `rewrite_enabled=true`、`template_id=\"correct\"`；
  - 但之后多次热键任务的 `CMD.start_transcribe_recording_base64.ctx` 仍为 `rewrite_enabled=false`、`template_id=null`；
  - 对应 `TASK.rewrite_effective` 全部 `skipped`（`rewrite_requested=false`）。
- 该现象发生在最新代码与最新进程下（Windows repo `HEAD=1df910c`），说明“现有修复尚未完全闭环”。

UNCONFIRMED

- 尚未定位到最终根因；当前只确认“设置读取结果”与“任务启动参数”之间仍有链路不一致。
- 推荐下一步验证动作（按优先级）：
  - 在 `MainScreen` 录音启动前后增加临时 trace（仅记录 `rewriteEnabledRef.current/templateIdRef.current`），与 `CMD.start_transcribe_recording_base64` 对齐；

## 更正与最新状态（2026-02-11，单一状态源与去兜底第一阶段）

VERIFIED

- 已完成“任务启动 rewrite 参数”去前端副本化：
  - `start_transcribe_recording_base64` 与 `start_transcribe_fixture` 不再接收前端传入的 `rewrite_enabled/template_id`；
  - 后端在命令入口统一从 `settings.json` 严格读取并解析（`settings::load_settings_strict + resolve_rewrite_start_config`），再注入 `TaskManager::StartOpts`。
- 已完成热键配置去默认兜底：
  - `hotkeys.rs` 移除 `unwrap_or(true/F9/F10)`；
  - 改为必须显式配置 `hotkeys_enabled/hotkey_ptt/hotkey_toggle`，否则记录 `E_HK_CONFIG`。
- 已完成 LLM 配置去默认兜底：
  - `llm.rs` 不再默认 `https://api.openai.com/v1` 和 `gpt-4o-mini`；
  - 缺失时返回 `E_LLM_CONFIG_*` 并在 `LLM.rewrite` span 记为 `E_LLM_CONFIG`。
- 已完成模板加载去内置回退：
  - `templates::load_templates` 在 `templates.json` 缺失时直接报 `E_TPL_FILE_NOT_FOUND`，不再自动回退内置模板。
- 前端已同步收敛：
  - `MainScreen` 不再缓存并传递 rewrite 参数；
  - `App` 不再在 `get_settings` 失败时设置 `{}`；改为显式 `settingsError`；
  - `SettingsScreen` 去掉 `F9/F10` 与布尔隐式默认，保存前增加必填校验。
- 验证结果：
  - `apps/desktop`: `npm run build` 通过。
  - `apps/desktop/src-tauri`: `cargo test -q` 通过（14 tests）。

UNCONFIRMED

- Windows 实机热键连续录音回归尚未在本轮执行（需复核 `trace.jsonl` 中 `CMD.start_transcribe_recording_base64` 与 `TASK.rewrite_effective`）。

## 更正与最新状态（2026-02-11，Windows 必做步骤已补齐）

VERIFIED

- 已按要求补齐 Windows 侧“同步最新源码 -> 重新编译 -> Debug 模式拉起”：
  - Windows 工作副本 `D:\\Projects\\TypeVoice` 已从 `1df910c` fast-forward 到 `91a6e77`。
  - Windows 编译闸门已执行并通过：
    - 命令：`scripts/windows/windows_compile_gate.ps1`
    - 结果：`PASS`（`cargo check --locked` 成功）。
  - 已清理旧的 TypeVoice 开发进程并拉起最新 `npm run tauri dev` 会话。
  - 进程复核：存在最新 `node/cargo/typevoice-desktop` 三件套。
  - 连通性复核：Windows 本机 `http://localhost:1420` 返回 `200`。

UNCONFIRMED

- 本轮尚未完成“连续热键录音 + trace 对账”的专项回归（进程与编译均已就位，可直接执行）。

## 更正与最新状态（2026-02-11，热键失效根因与修复）

VERIFIED

- 热键失效根因不是系统快捷键权限，而是配置与当前严格模式不匹配：
  - `settings.json` 中 `hotkeys_enabled/hotkey_ptt/hotkey_toggle/hotkeys_show_overlay` 为 `null`；
  - 启动时 `HK.apply` 报错 `E_HK_CONFIG`，错误链为 `E_SETTINGS_HOTKEYS_ENABLED_MISSING`。
- 已完成修复：
  - 路径：`D:\\Projects\\TypeVoice\\tmp\\typevoice-data\\settings.json`
  - 变更：`hotkeys_enabled=true`、`hotkey_ptt=\"F9\"`、`hotkey_toggle=\"F10\"`、`hotkeys_show_overlay=true`
  - 已重启 Windows `tauri dev` 会话并验证：
    - `trace.jsonl` 出现 `HK.apply status=ok`（`ctx.enabled=true, ptt=F9, toggle=F10`）
    - `http://localhost:1420` 返回 `200`
  - 检查 `App.reloadSettings` 的 `catch -> setSettings({})` 是否在某些时序下覆盖了有效设置；
  - 对比 UI 按钮路径与热键路径在同一会话下的启动参数差异，确认是否仅热键受影响。

## HANDOFF SNAPSHOT（2026-02-12 00:30 CST，可直接接班）

### 当前有效目标（与 SPEC 对齐）

VERIFIED

- 保持 MVP 主链路稳定：`Record -> Preprocess -> Transcribe -> Rewrite(可选) -> Persist -> Copy`，其中 rewrite 由后端单点读取 settings 决定，不允许前端/热键路径再出现状态副本漂移。对齐 `docs/memory/SPEC.md` 第 1/2/4 节。
- 保持“可诊断性”硬约束：失败必须能在落盘日志中定位（`step_id + code + error chain/backtrace`），不能依赖控制台输出。对齐 `docs/memory/SPEC.md` 第 4.1 节。

### 当前状态（Done / Now / Next）

#### Done

VERIFIED（本轮已复核）

- `AGENTS.md` 索引已按技能脚本更新并通过检查：
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py`
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --check`
- 最新运行日志显示 hotkey + rewrite 链路可用：
  - `trace.jsonl`：`HK.apply status=ok`（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl:1501`、`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl:1503`）。
  - `trace.jsonl`：任务启动参数来自 settings，且 rewrite 打开（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl:1513`）。
  - `trace.jsonl`：`TASK.rewrite_effective rewrite_entered=true`（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl:1533`）。
  - `trace.jsonl`：`LLM.rewrite status=ok, status=200`（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl:1540`）。
  - `metrics.jsonl`：`Rewrite completed -> task_done`（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl:1116`、`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl:1120`）。

#### Now

VERIFIED

- rewrite 消失/不生效主问题当前可复现为“已恢复”，不再是持续阻断项。
- 当前仍存在非阻断噪声样本：
  - ASR 空文本失败（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl:1109`，`E_ASR_FAILED`）。
  - 上一窗口截图偶发零尺寸（`/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl:1479`，`E_SCREENSHOT window has zero size`）。

#### Next

VERIFIED（建议执行顺序）

1. 连续做 5~10 次热键触发回归，确认每次 `CMD.start_transcribe_recording_base64.ctx.rewrite_enabled/source/template_id` 都稳定（尤其 `source="settings"`）。
2. 针对 ASR 空文本做最小复现采样（短语音/静音/噪声边界），按 `task_id` 聚合 trace+metrics，区分“输入问题”与“ASR runner 返回异常”。
3. 若截图零尺寸频率升高，再进入选窗策略优化；若低频且不阻断 rewrite，则继续保持 best-effort。

### 当前工作集（关键文件路径、关键命令、关键约束）

#### 关键文件路径

VERIFIED

- `apps/desktop/src-tauri/src/settings.rs`（严格配置解析：rewrite/hotkey）
- `apps/desktop/src-tauri/src/hotkeys.rs`（`HK.apply` 与热键配置错误码）
- `apps/desktop/src-tauri/src/lib.rs`（命令入口，rewrite 启动参数来源）
- `apps/desktop/src/screens/MainScreen.tsx`（热键监听生命周期与前端录音触发）
- `apps/desktop/src-tauri/src/context_capture.rs`
- `apps/desktop/src-tauri/src/context_capture_windows.rs`
- `apps/desktop/src-tauri/src/llm.rs`
- `/mnt/d/Projects/TypeVoice/tmp/typevoice-data/settings.json`（当前运行时配置）
- `/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl`
- `/mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl`

#### 关键命令

VERIFIED

- 更新/校验索引：
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py`
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --check`
- 日志快查：
  - `rg -n 'HK.apply|CMD.start_transcribe_recording_base64|TASK.rewrite_effective|LLM.rewrite|E_ASR_FAILED' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl`
  - `rg -n 'task_done|Rewrite|E_ASR_FAILED' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl`
- 运行态配置核对：
  - `jq '{hotkeys_enabled,hotkey_ptt,hotkey_toggle,hotkeys_show_overlay,rewrite_enabled,rewrite_template_id}' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/settings.json`

#### 关键约束

VERIFIED

- 单一状态不允许副本：rewrite/hotkey 运行态只以后端 `settings` 解析结果为准。
- 不允许静默兜底：缺失配置直接报稳定错误码（例如 `E_HK_CONFIG` / `E_SETTINGS_*`），不再隐式默认。
- 回归判定必须“按 task_id 看完整链路”，不能只看日志末尾是否出现任意 `err/failed`。

### 风险与坑（本阶段新增/高复发，指向 PITFALLS）

VERIFIED

- 旧版或损坏的 `settings.json` 含 `null` 热键字段时，启动会报 `E_HK_CONFIG`，表现为“快捷键全部失效”；见 `docs/memory/PITFALLS.md` 新增条目“热键配置为 null 导致 HK.apply 失败（E_HK_CONFIG）”。
- trace/metrics 是多任务并发交织写入，若不按 `task_id` 聚合，容易把“上一条失败”误判为“当前测试失败”；见 `docs/memory/PITFALLS.md` 新增条目“并发日志交织导致误判（需按 task_id 聚合）”。
- `E_ASR_FAILED: Empty ASR text` 仍会偶发，容易被误认为 rewrite 失效；见 `docs/memory/PITFALLS.md` 既有 ASR 相关条目与本轮补充。

### 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- 当前 `settings.json` 同时存在 `rewrite_template_id` 与 `active_template_id:null`，运行时是否会长期只读前者（兼容字段策略是否需清理）。
  - 推荐验证：在 `settings.rs` 明确字段优先级，并增加单测覆盖“旧字段/空字段/混合字段”。
- `E_ASR_FAILED: Empty ASR text` 的主因占比（静音输入 vs runner 偶发）尚无量化。
  - 推荐验证：固定 20 次短样本（含静音/低音量/正常语音）并统计失败分布，记录到 `metrics`。
- `E_SCREENSHOT window has zero size` 当前作为 best-effort 非阻断处理，是否需要升为可配置开关尚未决策。
  - 推荐验证：统计一周内该错误频次与对 rewrite 质量影响，再决定是否调整选窗策略。

## 更正与最新状态（2026-02-12，AGENTS 索引排除集已纠正）

VERIFIED

- 已按项目完整排除目录集重建 `AGENTS.md` 索引，恢复对 `fixtures/metrics/models/tmp/target` 等目录的忽略。
- 已使用同一组 `--exclude-dir` 参数执行 `--check` 校验通过。

UNCONFIRMED

- 若后续通过不同命令再次更新索引，存在“回退到默认排除集”的复发风险。
- 推荐验证：每次更新后固定执行同参 `--check`，并人工抽查 `AGENTS.md` 中 `exclude_dirs` 行。

## HANDOFF SNAPSHOT REFRESH（2026-02-12 01:18 CST，清上下文前）

### 当前有效目标（与 SPEC 对齐）

VERIFIED

- 维持 Windows MVP 主链路稳定：`Record -> Preprocess -> Transcribe -> Rewrite(可选) -> Persist -> Copy`，并保持“单一状态源 + 去兜底”约束持续生效。
- 维持可诊断性硬约束：所有失败必须可在落盘日志按 `step_id/code/task_id` 定位。

### 当前状态（Done / Now / Next）

#### Done

VERIFIED

- 已按 `agents-md-project-index` 技能重新执行索引更新，并使用项目完整排除目录集完成同参 `--check`。
- `AGENTS.md` 当前 `exclude_dirs` 保持为完整项目集合（含 `fixtures/metrics/models/tmp/target` 等）。
- 当前仓库在本次刷新后无新增代码改动，仅记忆文档会更新。

#### Now

VERIFIED

- rewrite 消失/不生效主问题处于“已恢复”状态；最近成功样本已覆盖 hotkey + rewrite + task_done 完整链路。
- 当前高频非阻断噪声仍是 `E_ASR_FAILED: Empty ASR text` 与偶发 `E_SCREENSHOT window has zero size`。

#### Next

VERIFIED（建议接班顺序）

1. 用 5~10 次热键触发做稳定性回归，按 `task_id` 核对每次是否进入 rewrite。
2. 对 `E_ASR_FAILED` 做分桶统计（静音/低音量/正常语音），确认主要根因占比。
3. 若截图零尺寸频率持续升高，再进入选窗策略优化；否则维持 best-effort。

### 当前工作集（关键文件路径、关键命令、关键约束）

#### 关键文件路径

VERIFIED

- `apps/desktop/src-tauri/src/settings.rs`
- `apps/desktop/src-tauri/src/hotkeys.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src/screens/MainScreen.tsx`
- `apps/desktop/src-tauri/src/context_capture_windows.rs`
- `apps/desktop/src-tauri/src/llm.rs`
- `/mnt/d/Projects/TypeVoice/tmp/typevoice-data/settings.json`
- `/mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl`
- `/mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl`

#### 关键命令

VERIFIED

- 索引更新/校验（必须同参）：
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --exclude-dir ...`
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --check --exclude-dir ...`
- 日志核对：
  - `rg -n 'CMD.start_transcribe_recording_base64|TASK.rewrite_effective|LLM.rewrite|E_ASR_FAILED' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/trace.jsonl`
  - `rg -n 'task_done|Rewrite|E_ASR_FAILED' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/metrics.jsonl`
- 配置核对：
  - `jq '{hotkeys_enabled,hotkey_ptt,hotkey_toggle,hotkeys_show_overlay,rewrite_enabled,rewrite_template_id}' /mnt/d/Projects/TypeVoice/tmp/typevoice-data/settings.json`

#### 关键约束

VERIFIED

- 结论必须绑定单一 `task_id` 完整链路，禁止跨任务拼接证据。
- 运行态配置问题优先排查 data dir 的 `settings.json`，不要先假设系统热键权限问题。
- 索引维护必须使用完整排除目录集，禁止回退脚本默认排除。

### 风险与坑（本阶段新增或高概率复发）

VERIFIED

- 见 `docs/memory/PITFALLS.md`：
  - “热键配置字段为 null 导致 `E_HK_CONFIG`”
  - “trace/metrics 多任务交织导致误判（需按 `task_id` 聚合）”
  - “`E_ASR_FAILED: Empty ASR text` 易被误判为 rewrite 故障”

### 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- `rewrite_template_id` 与 `active_template_id` 的兼容字段策略是否需要统一收敛。
  - 推荐验证：在 `settings.rs` 增加兼容优先级单测并固化行为。
- `E_ASR_FAILED` 失败样本是否主要来自输入静音边界。
  - 推荐验证：固定 20 次样本并记录失败分布。
- `E_SCREENSHOT window has zero size` 是否需要升级为显式配置开关。
  - 推荐验证：按周统计频率与对 rewrite 质量影响再决策。

## 更正与最新状态（2026-02-12，热键“按下瞬间截图”链路已落地）

VERIFIED

- 已完成实现：热键按下时由后端立即抓取当前前台窗口截图，并生成一次性 `capture_id`。
  - `tv_hotkey_record` 事件新增字段：`capture_id/capture_status/capture_error_code/capture_error_message`。
  - 前端仅在 `capture_status=ok` 且有 `capture_id` 时才会启动热键录音。
- 已完成实现：`start_transcribe_recording_base64` 新增 `capture_id/capture_required` 入参。
  - 当 `capture_required=true` 时，后端会强制校验 `capture_id`，缺失/过期/无截图直接报错：
    - `E_CONTEXT_CAPTURE_REQUIRED`
    - `E_CONTEXT_CAPTURE_NOT_FOUND`
    - `E_CONTEXT_CAPTURE_INVALID`
- 已完成实现：任务流水线支持注入预采集上下文（`StartOpts.pre_captured_context`）。
  - 注入后会关闭运行时“上一窗口截图”采集并写入 `CTX.hotkey_capture_injected`，避免窗口漂移到后时刻快照。
- 已完成实现：Windows 抓图链路新增黑帧校验（`validate_pixels`），当 `GetDIBits` 结果近似全黑时直接失败（`E_HOTKEY_CAPTURE`），不再把黑图当有效截图发送。
- 本地验证通过：
  - `cargo check --locked`（`apps/desktop/src-tauri`）
  - `cargo test --locked -q`（14 tests passed）
  - `npm run build`（`apps/desktop`）

UNCONFIRMED

- 尚未完成 Windows 实机回归来最终确认“按下瞬间窗口 = 最终发送截图窗口”在多场景（切窗/任务切换器/Chrome）下 100% 一致。
- 当前截图底层仍使用 `PrintWindow/GetDIBits`，虽已改为“按下瞬间句柄”采集，但平台级黑帧兼容性是否完全消除仍需 Windows 实测数据确认。

## 更正与最新状态（2026-02-12，Windows 黑图根因已定位）

VERIFIED

- 已完成“最近几次 Windows debug 日志 + 图片”专项复核，黑图问题根因已定位且可复核：
  1. 选窗阶段会把 Shell/任务切换器/任务栏等窗口当作“上一外部窗口”（`ForegroundTracker` 仅排除本进程 PID，无窗口类型过滤）。
  2. 抓图阶段 `PrintWindow/GetDIBits` 即使返回成功，也可能得到全黑像素；当前实现会把这类结果判定为 `status=ok` 并继续进入 LLM 请求。
- 证据链（同一数据目录）：
  - `debug/*/prev_window.png` 10 张样本中 7 张全黑或黑条（`YAVG=16`，且重复哈希）。
  - `trace.jsonl` 里 `CTX.prev_window.screenshot status=ok` 共 38 条，其中 27 条落在 3 个黑图哈希（`c479...`/`fb2d...`/`eac3...`）。
  - 黑图样本对应的 `PREVIOUS WINDOW` 多次为 `title=任务切换`、`process=C:\\Windows\\explorer.exe`；另有 `Chrome` 窗口样本也出现全黑，说明不仅是“选到 explorer”单一原因。
- 结论边界：
  - 该问题已定位为“Windows 选窗策略 + PrintWindow 平台兼容性”组合问题；
  - 不是 `debug` 图片保存路径、PNG 编码、或 trace 记录链路导致的伪黑图。

UNCONFIRMED

- 最近 16 次 `fb2d...` 高频黑图样本在当前 trace 中仅有 `has_title/has_process` 布尔信息（未含具体标题/进程字符串）；若要进一步细分触发窗口类型，需要在相同复现场景下再次开启截图 debug 落盘采样。

## HANDOFF SNAPSHOT REFRESH（2026-02-12 14:39 CST，清上下文前）

### 当前有效目标（与 SPEC 对齐）

VERIFIED

- 维持 MVP 主链路稳定：`Record -> Preprocess -> Transcribe -> Rewrite -> Persist -> Export`，并保持 Windows 端可运行、可诊断、可回归。
- 对热键链路继续坚持“按下瞬间截图必须对应正确窗口”的正确性目标：不允许再退回“截图失败就无图继续”的策略。
- 交付口径保持不变：所有“可用/不可用”结论都必须绑定单一 `task_id` 的完整证据链。

### 当前状态（Done / Now / Next）

#### Done

VERIFIED

- `AGENTS.md` 索引已按 `agents-md-project-index` 刷新，并使用项目完整排除目录集；`--check` 已通过。
- 主仓当前为干净工作区：`git status --short --branch` 仅返回 `## main`。
- 主仓最新提交为 `b499006`（Windows 编译修复：`WindowInfo` 增加 `#[derive(Debug, Clone)]`）。
- Windows dev 会话当前可用：
  - 进程存在 `typevoice-desktop.exe`（PID 28036）与 `node`；
  - `Invoke-WebRequest http://localhost:1420` 返回 `200`。
- 本轮 memory 文件已刷新：`CONTINUITY.md`、`PITFALLS.md`、`DECISIONS.md`。

#### Now

VERIFIED

- Windows 工作副本（`/mnt/d/Projects/TypeVoice`）尚未与主仓对齐：
  - `HEAD=3a238b1`（落后主仓 `b499006`）；
  - 存在本地改动 `M apps/desktop/src-tauri/src/context_capture_windows.rs` 与未跟踪 `?? .cache/`。
- 当前 Windows 运行进程来自 `D:\Projects\TypeVoice`，因此运行态可能混入未提交改动，不满足“运行态=已提交源码”要求。

#### Next

VERIFIED（建议接班顺序）

1. 在 Windows 副本先处理脏工作区（提交/暂存/清理由接班者按现场策略执行），恢复可 fast-forward 状态。
2. 将 `/mnt/d/Projects/TypeVoice` fast-forward 到主仓 `b499006`，并复核 `HEAD` 一致。
3. 清理旧 `tauri dev` 会话后只保留一个最新会话，再做热键截图正确性回归（按 `task_id` 聚合证据链）。

### 当前工作集（关键文件路径、关键命令、关键约束）

#### 关键文件路径

VERIFIED

- `AGENTS.md`
- `docs/memory/CONTINUITY.md`
- `docs/memory/PITFALLS.md`
- `docs/memory/DECISIONS.md`
- `apps/desktop/src-tauri/src/context_capture.rs`
- `apps/desktop/src-tauri/src/context_capture_windows.rs`
- `apps/desktop/src-tauri/src/hotkeys.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/src/task_manager.rs`
- `apps/desktop/src/screens/MainScreen.tsx`

#### 关键命令

VERIFIED

- 更新索引（完整排除集）：
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --root /home/atticusdeng/Projects/TypeVoice --agents-md /home/atticusdeng/Projects/TypeVoice/AGENTS.md --exclude-dir ...`
- 校验索引：
  - `python /home/atticusdeng/.agents/skills/agents-md-project-index/scripts/update_agents_md_project_index.py --root /home/atticusdeng/Projects/TypeVoice --agents-md /home/atticusdeng/Projects/TypeVoice/AGENTS.md --exclude-dir ... --check`
- 双仓状态核对：
  - `git status --short --branch`
  - `git -C /mnt/d/Projects/TypeVoice status --short --branch`
  - `git -C /mnt/d/Projects/TypeVoice rev-parse --short HEAD`
- Windows 运行态核对：
  - `powershell.exe -NoProfile -Command "Get-Process typevoice-desktop,node ..."`
  - `powershell.exe -NoProfile -Command "(Invoke-WebRequest -UseBasicParsing http://localhost:1420).StatusCode"`

#### 关键约束

VERIFIED

- 用户明确要求：按下热键瞬间必须截到正确窗口，不接受“错误窗口过滤 + 无图继续”类降级行为。
- 在 Git 仓库完成任务要求原子化提交。
- 更新 AGENTS 索引必须使用项目完整排除目录集，不允许回退脚本默认排除集。
- 涉及 Windows-only 路径的改动，交付前必须做 Windows 实机编译复核。

### 风险与坑（本阶段新增或高概率复发，指向 PITFALLS）

VERIFIED

- Linux 侧检查通过不代表 Windows 可编译：见 `docs/memory/PITFALLS.md` 新增条目“Windows-only 编译才暴露 trait 派生缺失”。
- Windows 副本有脏改动会阻断 fast-forward，同步失败后运行态易与主仓分叉：见 `docs/memory/PITFALLS.md` 新增条目“Windows 副本存在本地改动时，fast-forward 同步会被阻断”。
- 热键截图链路结论仍需按单一 `task_id` 验证，避免并发日志交织误判：见 `docs/memory/PITFALLS.md` 既有条目“trace/metrics 多任务交织，若不按 task_id 聚合会产生误判”。

### 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- Windows 当前运行会话是否完全对应主仓 `b499006`（而不是 `3a238b1` + 本地补丁）。
  - 推荐验证：先清理 `/mnt/d` 工作区并 fast-forward，再重启单实例 `tauri dev`，复核进程路径与 `git rev-parse --short HEAD` 一致。
- “按下热键瞬间窗口 == 最终发送截图窗口”在高频切窗场景是否达到用户要求的稳定正确。
  - 推荐验证：固定场景连续 20 次录制，逐次记录 `task_id`、`capture_id`、`prev_window.png` 与当时前台窗口，按任务闭环判定。
- 黑帧检测（`validate_pixels`）在真实复杂窗口（浏览器硬件加速、系统 UI）下的误拒率是否可接受。
  - 推荐验证：在同一台 Windows 机器开启 debug 落盘，统计“判黑拒绝”与“实际可用截图”差异后再决策阈值。
