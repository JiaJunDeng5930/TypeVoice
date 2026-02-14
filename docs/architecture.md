# TypeVoice 架构（Windows MVP）

目标：在满足 `docs/base-spec.md` 与 `docs/verification.md` 的前提下，给出可实现、可取消、可度量、可扩展（热键/托盘/自动输入）的最小架构。

## 1. 分层与依赖方向（冻结）

采用“端口-适配器（Hexagonal）”思路：

- Presentation（Tauri + Web UI）
  - 只负责交互与展示
  - 不直接处理音频/模型/网络细节
- Application（Use Cases / Pipeline Orchestrator）
  - 负责把 Record/Preprocess/Transcribe/Rewrite/Persist/Export 这些阶段串起来
  - 负责取消、状态机、事件与指标
- Adapters（实现层）
  - FFmpeg 调用适配器
  - ASR Runner 适配器（PyTorch CUDA）
  - LLM API 适配器
  - Storage（历史记录/模板/配置/安全存储）
  - Platform（剪贴板、未来的热键/托盘/自动输入）

依赖约束：

- Presentation 依赖 Application（通过 Tauri commands/events）
- Application 只依赖 Ports（trait/interface）
- Adapters 实现 Ports

## 2. 关键组件与职责

### 2.1 PipelineOrchestrator（应用层核心）

职责：

- 接收“开始录音/结束录音/取消任务/用 fixtures 运行验证”等命令
- 驱动阶段状态机，产出 UI 事件与结构化 metrics
- 串行/并行策略：MVP 以串行为主，避免资源竞争
- 任务隔离：同一时间最多 1 个 active 任务（MVP 冻结，后续可扩展为队列）

输入：

- 录音输入（来自 Recorder）或 音频文件路径（fixtures / 用户导入）
- 用户配置（模型路径、模板、LLM 开关等）

输出：

- Task events（阶段变更、进度、完成/失败）
- Task result（asr_text、final_text、耗时、rtf、错误码）

统一入口约束（新增）：

- 对外仅暴露单一任务启动命令 `start_task(req)`。
- `req` 仅表达触发意图与输入模式（例如 `trigger_source`、`record_mode`、`recording_asset_id`），不携带可变执行策略。
- 热键与 UI 必须走同一条 Orchestrator 路径，不允许并行实现第二套任务入口。
- 录音停止命令不得直接起任务；只返回受管控的录音资产句柄（`recording_asset_id`），再由 `start_task(req)` 进入统一编排。
- 录音资产句柄必须由后端注册并托管生命周期（租约到期自动回收），禁止 UI 直接传入文件路径触发任务。

### 2.2 Recorder（适配器）

职责：

- 提供录音音频文件（临时文件）
- 对外隐藏平台差异（Windows 设备枚举等）

实现约束（新增）：

- 主录音链路由后端命令托管，前端不直接持有录音数据生命周期。
- 推荐命令形态：`start_backend_recording` / `stop_backend_recording` / `abort_backend_recording`，其中 `stop_backend_recording` 仅结束采集并返回 `recording_asset_id`。

注意：录音实现与 ASR 解耦，验证与性能测量默认走 fixtures（不依赖录音）。

### 2.3 FFmpegAdapter（适配器）

职责：

- 定位内置 `ffmpeg.exe`
- 将输入音频统一转为 ASR 需要的格式（建议 wav/pcm/16k/mono，最终以 ASR runner 要求为准）
- 可取消（需要能 kill ffmpeg 进程）
- 产出：标准化音频文件路径 + metrics（elapsed_ms、exit_code）

### 2.4 AsrAdapter（适配器，冻结选择：PyTorch CUDA）

职责：

- 管理 ASR Runner 的生命周期与调用
- 约束：不允许降级到 CPU
  - 若 cuda 不可用或 runner 返回 cpu，视为失败（错误码需明确）
- 产出：text + metrics（rtf、device_used、elapsed_ms）

进程边界（建议）：

- ASR Runner 为独立进程（Python），避免 Rust 侧直接链接 PyTorch 的复杂度。
- IPC 建议：stdin/stdout JSON（单请求单响应）或本地 HTTP（若需要并发）。

### 2.5 LlmRewriteAdapter（适配器）

职责：

- 将 asr_text + 模板渲染后的 prompt 发给 API
- 失败必须回退：保留 asr_text，final_text=asr_text，并返回结构化错误
- 默认验证不调用真实 API（见 `docs/verification.md`）

### 2.6 TemplateStore / SettingsStore / HistoryStore（适配器）

职责：

- 模板：增删改查、导入导出（JSON）、模板版本快照（至少保存模板名+hash）
- 设置：非敏感配置落盘
- API Key：走 Windows 安全存储（Credential Manager）或等价方案
- 历史：仅文本与元信息（不保存音频）

### 2.7 MetricsSink（适配器）

职责：

- 追加写入结构化 metrics（建议 JSONL 一行一条）
- quick/full 验证只需要控制台摘要 + JSONL 指标，不生成长报告

## 3. 核心端口（Ports）建议

为保证可测试与可替换，应用层依赖以下抽象：

- `AudioPreprocessorPort`
  - `preprocess(input_path) -> output_path + metrics`
- `AsrPort`
  - `transcribe(audio_path, params) -> text + metrics`
- `RewritePort`
  - `rewrite(text, template, params) -> text + metrics`
- `HistoryPort`
  - `append(record)`
  - `list(limit)`
- `TemplatePort`
  - `list/get/upsert/delete/import/export`
- `ClipboardPort`
  - `copy(text)`
- `MetricsPort`
  - `append(event)`

### 3.1 当前实现约束（可测试性）

- `TaskManager` 必须通过可注入依赖访问外部系统：
  - `AsrClient`（ASR 启动/重启/转写）
  - `ContextCollector`（上下文抓取）
  - `TaskManagerDeps`（FFmpeg 预处理、模板读取、历史写入、指标写入）
- 应用编排代码不得直接硬编码外部依赖构造，默认实现由 `TaskManager::new()` 装配，测试可注入替身实现。
- 前端屏幕组件不得直接绑定 Tauri API；必须通过运行时端口访问：
  - `TauriGateway`（命令调用与事件订阅）
  - `TimerPort`（定时器）
  - `ClipboardPort`（剪贴板）

## 4. Gate 驱动的关键实现约束

这些是为了让 `quick/full` Gate 可实现而强制的架构约束：

- 所有阶段必须产出结构化 metrics（至少：task_id、stage、elapsed_ms、result_code、device_used、rtf）。
- 取消必须是跨阶段的一等公民（CancelToken + 可终止外部进程）。
- fixtures 路径约定必须固定（`fixtures/zh_10s.ogg` 等），且通过 FFmpeg 预处理统一格式，避免样本格式差异影响结果。
