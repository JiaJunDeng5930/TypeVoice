# PITFALLS（踩坑与经验）

- 本文件只保留可复用、可执行的踩坑项，不保留单次排障日志与历史证据。

## 1. 诊断口径

- Windows release 无稳定控制台时，不能只看 stdout/stderr；所有失败必须在落盘 trace 中可复现。
- 运行结论不能按 `tail` 末尾单行判断，必须按 `task_id` 汇总完整链路。

## 2. FFmpeg 与工具链

- 在 Windows 上若 `ffmpeg` 前置可执行不存在，任务会在入口 preflight 阶段失败；应以仓库 toolchain 与清单方式补齐，而非依赖外部猜测安装。
- 未清理旧会话重新拉起 `tauri dev` 时，热键和文件锁问题可能被放大，需先清会话再验证。
- ASR runner 在 ready 前若返回 `ok=false/error.code`，上层若仅等待 `asr_ready` 会误报 EOF；必须优先识别结构化错误行并透传错误码。

## 3. 环境与路径

- WSL 下调用 Windows 命令时，不能默认信任父 shell 的 PATH；需确认 `cargo`、`node`、`ffmpeg` 在调用链可见。
- PowerShell 参数中 `$`、反引号等特殊字符需在传递层面转义，避免命令在执行前被当前 shell 污染。
- `run-latest.ps1` 若报 `tauri-latest-run.txt is being used by another process`，说明旧会话未清理干净；先执行文档中的停止命令清理 `typevoice-desktop.exe` 与 `node.exe`，再重跑同一命令。
- `verify_quick.py` 会在本地模型缺失时直接失败（`models/Qwen3-ASR-0.6B`）；在无模型环境中先补齐模型资产再跑 gate，避免把环境缺失误判为代码回归。
- `verify_quick.py` 在仓库根目录缺失 `./.venv` 时会直接失败（`FAIL: .venv missing`）；在执行 gate 之前先恢复项目本地虚拟环境。
- 用户要求“严格按文档执行”时，任何自行加步骤（哪怕是为了排障）都会被判定为偏移；必须先原样回传文档命令报错，再按文档明示步骤处理。

## 4. 全局热键链路

- 热键录音参数必须与 settings 一致，避免界面与热键路径出现独立配置副本。
- 任何任务可取消路径（cancel）都必须可观测地从 pre-cancel 到完成态关闭，不能停在中间态。
- Windows dshow 下 `audio=default` 在部分机器/驱动组合中不可用；录音输入应优先使用 dshow `Alternative name`（`audio="@device_cm_{...}\\wave_{...}"`）并在首次探测后固化，避免默认设备切换（如蓝牙连接）引入漂移。

## 5. 模板与设置

- `templates.json` 或 `settings.json` 的缺失/关键字段缺失应 fail-fast；禁止静默回退到旧默认导致“成功展示”与“配置不一致”同时存在。
- 进行改动前先校验模板与设置文件完整性，再触发 rewrite。

## 6. 日志与隐私

- Debug 级日志可用于排障，但默认关闭敏感内容采集；开启时仅写入项目内必要字段（路径尽量去标识化）。
- 任何自动化结论需保留最小上下文，避免把非事实的推断当结论。
- `trace.jsonl` 在并发写场景下需序列化写入；否则可能出现粘连行导致 JSON 解析失败。
- Windows 自动粘贴链路里，`SendInput` 返回成功并不等于“文本已进入目标输入框”；目标窗口/焦点判定必须与任务时刻一致，且不能依赖“最近外部窗口”推断来宣称成功。
- Windows 热键 overlay 若在导出时仍保持可见，可能抢占前台焦点，导致 Unicode 输入命中 TypeVoice 自身窗口并产生“假成功”；导出前先隐藏 overlay 并校验目标进程归属。
