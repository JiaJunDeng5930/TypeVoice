# TypeVoice 技术规格

状态：冻结中。

范围：Windows 桌面端的系统设计与工程约束。

## 1. 总体架构

系统由前端发送用户意图，后端核心状态机执行业务流转并输出状态快照。后端业务模块按职责拆分：

- `commands`：Tauri 命令入口。
- `voice_workflow`：核心业务状态机，统一推进录音转录、改写、插入和取消。
- `voice_tasks`：耗时异步任务执行层。
- `audio_capture`：录音会话、音频产物、系统采集适配。
- `transcription`：统一语音转录模块，选择本地或 API provider。
- `rewrite`：LLM 改写。
- `insertion`：复制和自动写入。
- `ui_events`：前端显示事件 actor。
- `settings`、`templates`、`history`、`obs`：配置、模板、历史、日志指标。

调用方向：

```text
Frontend
-> commands
-> voice_workflow
-> voice_tasks
-> audio_capture / transcription / rewrite / insertion
-> adapters
```

核心业务状态只由 `voice_workflow` 持有。设备监听、设备缓存、ASR runner、FFmpeg 进程、事件 mailbox 等边缘资源状态保留在对应模块。

异步结果统一先发给前端。前端收到 `stateChanging` 事件后调用 `workflow_apply_event`，状态机处理后返回 `WorkflowView`，前端据此更新主界面。

## 2. 命令规范

前端通过以下命令控制核心语音流程：

- `workflow_snapshot() -> WorkflowView`
- `workflow_command(req) -> WorkflowView`
- `workflow_apply_event(req) -> WorkflowView`

`workflow_command(req)` 的 `command` 取值：

- `primary`
- `rewriteLast`
- `insertLast`
- `copyLast`
- `cancel`

兼容命令仍由命令层转发到 `voice_workflow`：

- `record_transcribe_start(req) -> { sessionId }`
- `record_transcribe_stop(req) -> TranscriptionResult`
- `record_transcribe_cancel(req) -> void`
- `rewrite_text(req) -> RewriteResult`
- `insert_text(req) -> InsertResult`
- `transcribe_fixture(req) -> TranscriptionResult`

语义：

- 录音和转录属于同一个用户过程，由 start/stop/cancel 控制。
- 改写由用户单独触发。
- 插入由用户单独触发。
- 复制最近结果由用户单独触发。
- fixtures 转录走统一转录模块。
- 命令层只调用 `voice_workflow` 接受用户意图，并把返回的异步任务交给 `voice_tasks`。
- 前端显示使用 `WorkflowView`，其中包含当前阶段、会话 ID、最近结果文本、诊断文本和按钮可用性。
- 前端转发状态型异步事件时调用 `workflow_apply_event`。

核心状态：

- `Idle`
- `Recording`
- `Transcribing`
- `Transcribed`
- `Rewriting`
- `Rewritten`
- `Inserting`
- `Cancelled`
- `Failed`

## 3. 语音转录规范

统一转录模块负责：

- FFmpeg 预处理。
- provider 选择。
- 取消控制。
- 初始历史记录写入。
- 性能指标记录。
- ASR provider 生命周期管理。

Doubao provider：

- WebSocket 流式转录。
- 凭据通过 keyring 或环境变量读取。

远程 HTTP provider：

- 沿用当前 remote ASR 配置和 API Key 管理。
- 产出统一 `TranscriptionResult`。

## 4. 改写规范

- `rewrite_text` 使用 `transcriptId` 和输入文本执行改写。
- 模板来自设置或请求参数。
- 改写成功后更新历史记录。
- 改写失败返回结构化错误，调用方保留原始转录文本。
- hotkey 预采集上下文由 `voice_workflow` 保存，并在改写时一次性传入。

## 5. 插入规范

- `insert_text` 负责复制和自动写入。
- `copyLast` 只复制当前最终文本。
- 自动写入由设置项 `auto_paste_enabled` 控制。
- 自动写入失败时返回 `copied=true`、`autoPasteAttempted=true`、`autoPasteOk=false` 和错误信息。
- Windows 自动写入使用平台输入能力。
- Linux 自动写入使用 AT-SPI。

## 6. 事件规范

后端显示事件统一投递到 `ui_event`：

- `transcription.stage`
- `transcription.completed`
- `rewrite.completed`
- `audio.level`
- `diagnostic.error`
- `workflow.state`
- `workflow.task.failed`
- `workflow.task.cancelled`

事件包含：

- `effect`：`displayOnly` 或 `stateChanging`
- `eventId`：状态型事件去重 ID
- `sequence`：事件递增序号
- `taskId`：任务 ID
- `payload`：完成事件载荷

`displayOnly` 事件只影响界面显示，例如音频电平、阶段进度和后续流式文本片段。`stateChanging` 事件会由前端转发给 `workflow_apply_event`，例如转录完成、改写完成、插入完成、失败和取消。

`voice_tasks` 投递异步任务事件。`audio_capture` 只投递 `audio.level`。Tauri `AppHandle.emit` 只在 `ui_events` actor 中集中执行。

## 7. 存储规范

历史记录包含：

- `task_id`
- `created_at_ms`
- `asr_text`
- `final_text`
- `template_id`
- `preprocess_ms`
- `asr_ms`
- `rtf`
- `device_used`

转录完成时创建记录。改写完成时更新记录。音频中间产物默认清理。

## 8. 配置与密钥

普通配置写入 `settings.json`。API Key 使用系统安全存储或环境变量。

敏感字段不得写入日志。

## 9. 错误和日志

- 错误必须包含结构化错误码。
- UI 显示至少包含错误码和摘要。
- `trace` 和 `metrics` 继续由 `obs` 模块负责。
- 业务逻辑层避免吞掉错误，命令层负责控制响应输出。
