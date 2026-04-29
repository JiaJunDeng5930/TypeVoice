# TypeVoice 架构

目标：前端只发送用户意图并渲染后端状态快照；后端核心状态机统一控制录音转录、改写、插入和取消，并通过统一事件通道驱动显示。

## 1. 分层与依赖方向

- Frontend：React UI，只负责交互、显示和发送用户意图命令。
- Commands：Tauri 命令入口，负责参数映射、状态注入和调用核心状态机。
- Core Modules：`voice_workflow`、`voice_tasks`、`audio_capture`、`transcription`、`rewrite`、`insertion`、`ui_events`。
- Adapters：Doubao ASR、远程 HTTP ASR、FFmpeg、LLM API、平台输入、存储、系统音频设备。

依赖方向：

- 前端通过 typed backend client 调用 `workflow_snapshot`、`workflow_command` 和 `workflow_apply_event`。
- 命令层只推进 `voice_workflow`。
- `voice_workflow` 持有核心业务状态，并把耗时动作交给 `voice_tasks`。
- `voice_tasks` 调用录音、转录、改写、插入能力模块，并把异步结果投递给前端。
- 能力模块依赖端口或适配器。
- 前端收到状态型异步事件后调用 `workflow_apply_event`，状态机返回新的 `WorkflowView`。
- 显示事件和状态型事件都发入 `UiEventMailbox`，由 `ui_events` actor 统一投递 `ui_event`。
- `audio_capture` 继续直接投递 `audio.level`，因为音频电平是录音资源采样事件。

## 2. 核心模块

### 2.1 voice_workflow

职责：

- 作为后端唯一核心业务状态机。
- 持有当前 `phase`、会话 ID、录音 session ID、转录结果、改写结果、hotkey 预采集上下文和最近错误。
- 统一校验合法流转，非法操作返回结构化错误码。
- 用户命令进入进行中阶段后返回 `WorkflowView`。
- 接收前端转发的状态型事件，并据此完成、取消或失败当前任务。
- 用 `taskId` 和事件 ID 校验异步事件，避免过期结果覆盖当前状态。

状态：

- `Idle`
- `Recording`
- `Transcribing`
- `Transcribed`
- `Rewriting`
- `Rewritten`
- `Inserting`
- `Cancelled`
- `Failed`

### 2.2 audio_capture

职责：

- 管理录音会话生命周期。
- 管理录音音频产物和短期资产消费。
- 通过 Windows dshow 适配器采集音频。
- 通过 `UiEventMailbox` 投递音频电平事件。

状态机调用：

- `record_transcribe_start(req) -> { sessionId }`
- `record_transcribe_cancel() -> void`

### 2.3 voice_tasks

职责：

- 作为耗时异步任务执行层。
- 停止录音后调用转录模块。
- 调用改写和插入模块。
- 任务过程中投递 `displayOnly` 事件。
- 任务完成、失败、取消时投递 `stateChanging` 事件给前端。

### 2.4 transcription

职责：

- 提供统一语音转录能力。
- 管理预处理、取消、转录 provider 选择、历史初始写入、性能指标。
- 保留取消 token、子进程句柄等边缘资源状态。
- 依赖 Doubao provider 和远程 HTTP provider。

Provider：

- Doubao：WebSocket 流式语音转录 provider。
- Remote：HTTP API 语音转录 provider。

状态机调用：

- `record_transcribe_stop() -> TranscriptionResult`
- `transcribe_fixture(req) -> TranscriptionResult`

### 2.5 rewrite

职责：

- 独立执行文本改写。
- 读取 LLM 提示词、上下文和术语表。
- 接收 `voice_workflow` 传入的 hotkey 预采集上下文。
- 成功后更新同一条历史记录的 `final_text`。

状态机调用：

- `rewrite_text(req) -> RewriteResult`

### 2.6 insertion

职责：

- 统一管理复制和自动写入目标窗口。
- 自动写入失败时保留复制成功状态，并返回结构化错误。

状态机调用：

- `insert_text(req) -> InsertResult`

### 2.7 ui_events

职责：

- 提供 `UiEventMailbox`。
- 启动 actor，从 mailbox 读取事件并投递给前端 `ui_event`。
- 事件覆盖 workflow 状态快照、音频电平、任务进度、转录完成、改写完成、插入结果、取消和诊断错误。
- 每个事件包含 `effect`，取值为 `displayOnly` 或 `stateChanging`。

## 3. 前端交互

主屏幕只发送用户意图：

- 主按钮发送 `primary`，由 `voice_workflow` 按当前阶段决定开始、停止或取消。
- `REWRITE` 发送 `rewriteLast`，由 `voice_workflow` 选择最近一次 ASR 文本。
- `INSERT` 发送 `insertLast`，由 `voice_workflow` 选择当前最终文本。
- 点击最近结果文本发送 `copyLast`，由 `voice_workflow` 执行复制。

前端显示来自 `WorkflowView` 的按钮文案、禁用状态、最近结果和诊断文本。

前端事件处理：

- `displayOnly` 事件只更新界面过程显示。
- `stateChanging` 事件调用 `workflow_apply_event`。
- `workflow_apply_event` 返回的 `WorkflowView` 是主界面状态来源。

## 4. 数据契约

核心结果类型：

- `TranscriptionResult { transcriptId, asrText, finalText, metrics, historyId }`
- `RewriteResult { transcriptId, finalText, rewriteMs }`
- `InsertResult { copied, autoPasteAttempted, autoPasteOk, errorCode, errorMessage }`
- `WorkflowView { phase, taskId, recordingSessionId, lastTranscriptId, lastAsrText, lastText, lastCreatedAtMs, diagnosticCode, diagnosticLine, primaryLabel, primaryDisabled, canRewrite, canInsert, canCopy }`
- `UiEvent { kind, effect, eventId, sequence, taskId, stage, status, message, elapsedMs, errorCode, payload, tsMs }`

历史记录规则：

- 转录完成时创建历史记录，`final_text` 初始等于 `asr_text`。
- 改写完成时更新同一条历史记录。
- 插入只消费文本，不修改历史记录。

## 5. 验证约束

- 后端必须通过 `cargo check --locked --workspace`。
- 前端必须通过 `npm run build`。
- Rust 验证工具和后端 Rust 单测保持通过。
- Windows 一键网关仍作为功能实现后的最终验证入口。
