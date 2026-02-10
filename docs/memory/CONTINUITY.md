# CONTINUITY（外置记忆）

说明：

- 本文件记录“当前进度与工作集”，用于跨回合恢复上下文。
- 内容必须区分 `VERIFIED`（能用代码/文档/命令复核）与 `UNCONFIRMED`（推断/未验证）。
- 冻结规格的真源在 `docs/*.md`；本文件只维护“可执行接手摘要”，避免复述导致漂移（真源摘要：`docs/memory/SPEC.md`）。

## 当前有效目标（与 SPEC 对齐）

VERIFIED（截至 2026-02-10）

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

VERIFIED（截至 2026-02-10）

- 已确认“上一窗口截图可能全黑/很窄”的现象：属于 `PrintWindow` 对特定窗口（例如 Shell/任务栏/硬件加速/受保护 surface）返回空像素的已知兼容性问题；当前用户决定先忽略不修。详见 `docs/memory/PITFALLS.md`。
- `templates.json` 覆盖内置默认模板：若 data dir 存在 `templates.json`，应用会优先使用落盘模板，而不是 `default_templates()`。实现：`apps/desktop/src-tauri/src/templates.rs`。

### Next

VERIFIED（零上下文新 agent 可直接继续的推进顺序）

1. 固化“prompt 迭代闭环”：
   - 用 `scripts/llm_prompt_lab.py` 复现线上一次 rewrite 的输入结构（`### TRANSCRIPT` + `### CONTEXT`），快速对比不同 system prompt 的效果。
   - 当用户要求看“完整输入输出”，应直接粘贴脚本打印的完整 request/response（不要只给路径），同时保留落盘以便复查。
2. 确认“模板实际生效路径”：
   - 若想更新内置默认模板：改 `apps/desktop/src-tauri/src/templates.rs`。
   - 若想让 Windows 机器上的实际运行用新模板：必须更新该机器 data dir 内的 `templates.json`（或删除它让其回退默认；此行为会影响用户自定义，需谨慎）。
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
- prompt lab（示例，参数按环境填写）：
  - `python scripts/llm_prompt_lab.py --base-url "$TYPEVOICE_LLM_BASE_URL" --model "$TYPEVOICE_LLM_MODEL" --api-key "$TYPEVOICE_LLM_API_KEY" --system-prompt-file tmp/prompt.txt --transcript "..." --clipboard "..." --prev-process "C:\\\\Windows\\\\explorer.exe"`
- 从 trace 中定位某次任务的错误：
  - `jq -r 'select(.status==\"err\") | {ts_ms,task_id,stage,step_id,error,ctx} | @json' \"$TYPEVOICE_DATA_DIR/trace.jsonl\" | tail`

### 关键约束

VERIFIED

- 不增加 UI 预览/勾选流程；上下文采集与发送应自动完成（用户偏好：`docs/memory/USER_PREFS.md`）。
- 诊断信息必须落盘可定位（见 `docs/memory/SPEC.md` 的“可诊断性”硬约束），且不得落盘 API key 与截图像素/base64（除非显式 debug 开关允许）。
- 禁止手工维护 `file:line` 常量；定位依赖运行时 backtrace + 稳定 `step_id`/`code`。

## 风险与坑（指向 PITFALLS）

VERIFIED

- Windows release 无控制台：不要依赖 stdout/stderr；排障走 `trace.jsonl` 与 `debug/` 落盘。见 `docs/memory/PITFALLS.md`。
- `templates.json` 覆盖内置默认模板：提示词“看起来没生效”时优先检查 data dir。见 `docs/memory/PITFALLS.md`。
- 上一窗口截图可能全黑/空白：属于 WinAPI 兼容性与选窗策略交互，当前先忽略但会反复出现。见 `docs/memory/PITFALLS.md`。

## 未确认事项（UNCONFIRMED + 推荐验证动作）

UNCONFIRMED

- Windows 机器上的真实运行是否已开启 `TYPEVOICE_TRACE_ENABLED`（默认开启，但可能被环境覆盖）。
  - 推荐验证：在目标机器 data dir 查看 `trace.jsonl` 是否产生新行；若没有，检查环境变量。
- 选窗策略是否需要排除 Shell/任务栏窗口以降低“全黑/很窄截图”的概率。
  - 推荐验证：用 trace 对比 `CTX.prev_window.info` 记录的 `process_image` 与 `CTX.prev_window.screenshot` 的成功/尺寸分布，人工评估是否需要过滤。
