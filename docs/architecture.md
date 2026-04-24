# TypeVoice 架构

目标：前端显式控制录音转录、改写、插入；后端按业务能力拆成独立模块，并通过统一事件通道驱动显示。

## 1. 分层与依赖方向

- Frontend：React UI，只负责交互、显示和命令编排。
- Commands：Tauri 命令入口，负责参数映射、运行时检查和调用业务模块。
- Core Modules：`audio_capture`、`transcription`、`rewrite`、`insertion`、`ui_events`。
- Adapters：本地 ASR runner、API ASR、FFmpeg、LLM API、平台输入、存储、系统音频设备。

依赖方向：

- 前端通过 typed backend client 调用后端命令。
- 命令层依赖业务模块。
- 业务模块依赖端口或适配器。
- 会影响前端显示的模块只依赖 `UiEventMailbox`，由 `ui_events` actor 统一投递 `ui_event`。

## 2. 核心模块

### 2.1 audio_capture

职责：

- 管理录音会话生命周期。
- 管理录音音频产物和短期资产消费。
- 通过 Windows dshow 适配器采集音频。
- 通过 `UiEventMailbox` 投递音频电平事件。

命令入口：

- `record_transcribe_start(req) -> { sessionId }`
- `record_transcribe_cancel(req) -> void`

### 2.2 transcription

职责：

- 提供统一语音转录能力。
- 管理预处理、取消、转录 provider 选择、历史初始写入、性能指标。
- 依赖本地 provider 和 API provider。

Provider：

- Local：常驻 Python ASR runner，强制 CUDA。
- Remote：API 语音转录 provider，沿用当前 remote ASR 能力。

命令入口：

- `record_transcribe_stop(req) -> TranscriptionResult`
- `transcribe_fixture(req) -> TranscriptionResult`

### 2.3 rewrite

职责：

- 独立执行文本改写。
- 读取模板、上下文和术语表。
- 成功后更新同一条历史记录的 `final_text` 与 `template_id`。

命令入口：

- `rewrite_text(req) -> RewriteResult`

### 2.4 insertion

职责：

- 统一管理复制和自动写入目标窗口。
- 自动写入失败时保留复制成功状态，并返回结构化错误。

命令入口：

- `insert_text(req) -> InsertResult`

### 2.5 ui_events

职责：

- 提供 `UiEventMailbox`。
- 启动 actor，从 mailbox 读取事件并投递给前端 `ui_event`。
- 事件覆盖录音状态、音频电平、转录阶段、转录完成、改写完成、插入结果和诊断错误。

## 3. 前端编排

主屏幕交互拆成独立操作：

- 主按钮：开始录音转录、结束录音并转录、取消当前录音或转录。
- `REWRITE`：对最近一次转录结果执行改写。
- `INSERT`：插入当前显示文本。
- 点击最近结果文本：只执行浏览器剪贴板复制。

前端不再在转录完成后自动改写或自动插入。

## 4. 数据契约

核心结果类型：

- `TranscriptionResult { transcriptId, asrText, finalText, metrics, historyId }`
- `RewriteResult { transcriptId, finalText, rewriteMs, templateId }`
- `InsertResult { copied, autoPasteAttempted, autoPasteOk, errorCode, errorMessage }`

历史记录规则：

- 转录完成时创建历史记录，`final_text` 初始等于 `asr_text`。
- 改写完成时更新同一条历史记录。
- 插入只消费文本，不修改历史记录。

## 5. 验证约束

- 后端必须通过 `cargo check --locked`。
- 前端必须通过 `npm run build`。
- Python ASR 协议测试保持通过。
- Windows 一键网关仍作为功能实现后的最终验证入口。
