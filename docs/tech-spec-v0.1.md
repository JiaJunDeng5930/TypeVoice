# TypeVoice 技术规格 v0.1（冻结中）

状态：冻结中（“待确认/待验证”章节外，其余为实现约束）

范围：Windows 桌面端 MVP 的系统设计与工程约束。

## 1. 总体架构（MVP）

实现目标：将“录音、预处理、ASR、改写、历史记录、复制”组成可取消、可观察、可扩展的本地 Pipeline。

推荐模块划分（逻辑划分，不代表最终目录结构必须一致）：

- App Shell（Tauri + Web UI）
- UI 层：录音控制、状态展示、模板编辑、历史记录、设置页
- 后端桥接层：向 UI 暴露命令与事件（开始/结束/取消、进度、结果）
- Core Pipeline（Rust）
- 任务队列与状态机
- FFmpeg 调用与产物管理
- ASR Provider（本地，PyTorch CUDA）
- LLM Provider（在线 API）
- Storage（本地）
- 文本历史记录（SQLite 或等价持久化）
- 模板与设置（本地配置文件 + 安全存储）

## 2. Pipeline 规范

### 2.1 阶段定义

- Record：采集音频
- Preprocess：FFmpeg 预处理，产出标准化音频文件
- Transcribe：ASR 转录，产出文本
- Rewrite：调用 LLM API 改写（可选）
- Persist：写入历史记录（文本与元信息）
- Export：复制到剪贴板

### 2.2 可取消性

约束：任意阶段必须支持取消，取消应满足 `docs/base-spec-v0.1.md` 的交互延迟。

实现约束：

- 每个任务拥有唯一 `task_id`。
- Pipeline 内部每个阶段必须检查取消信号。
- 对外部进程的调用（FFmpeg、ASR Runner）必须能终止进程或终止正在执行的作业。

### 2.3 产物与缓存

- 音频中间产物为临时文件，任务完成后默认清理。
- 历史记录只保存文本与元信息，不保存音频。
- 若为了调试需要保留音频，必须是开发模式显式开关，且默认关闭。

## 3. FFmpeg 预处理规范（冻结）

### 3.1 分发形态

- Windows 安装包内置 `ffmpeg.exe`（具体路径与命名需要固定并可在运行时定位）。

### 3.2 输入输出约束

- 输入：录音输出的原始音频文件（容器/编码可由录音实现决定）。
- 输出：统一为 ASR 友好格式（建议 WAV/PCM 16kHz/mono，最终以 ASR Runner 要求为准）。

### 3.3 失败处理

- 若 FFmpeg 启动失败或返回非零退出码，必须将 stderr 摘要呈现到 UI，并给出“重新安装/修复”的诊断提示。

## 4. ASR 规范（冻结）

### 4.1 模型与后端

- 模型：`Qwen/Qwen3-ASR-0.6B`
- 后端：PyTorch CUDA（强制；不允许 CPU 降级）
- 允许依赖安装 CUDA 相关运行时与 VC++ Runtime（冻结）

### 4.2 ASR Runner 进程模型（建议约束）

为降低 Rust 侧直接链接 PyTorch 的复杂度，建议使用独立 ASR Runner 进程：

- 形态：单独可执行（或 Python 入口）由 Core Pipeline 启动与管理。
- 通信：stdin/stdout JSON RPC 或本地 HTTP（MVP 优先选择 stdin/stdout，减少端口与防火墙问题）。
- 约束：必须返回结构化结果与结构化错误码，禁止只输出人类可读日志。

### 4.3 输入输出接口（逻辑约束）

输入：

- `audio_path`：预处理后的音频文件路径
- `language`：默认 `zh`
- `device`：必须为 `cuda`
- `decode_params`：解码参数（需可配置，默认值在性能 Spike 后冻结）

输出：

- `text`：最终转录文本（含标点与分割，以模型输出为准）
- `segments`：可选（若模型支持），用于 UI 做更细粒度展示
- `metrics`
- `rtf`：RTF
- `audio_seconds`
- `elapsed_ms`
- `device_used`：cuda
- `model_id` 与 `model_version`

### 4.4 性能与降级

- 必须在 RTX 4060 Laptop 环境达成 `docs/base-spec-v0.1.md` 的 RTF 指标。
- 若 GPU 不可用必须失败（返回结构化错误码并给出诊断提示），不允许降级到 CPU。

## 5. LLM 改写规范（冻结）

### 5.1 功能目标

- 纠错：修正错字、标点、断句、术语统一，尽量不改变原意。
- 表达澄清：对含混、指代不明、口语省略处做更清晰表述，不得编造事实。

### 5.2 提示词模板系统（冻结）

- 模板必须可在 UI 编辑，不需要改源码。
- 模板至少包含：
- `id`：模板 id
- `name`：模板名
- `system_prompt`：system prompt 文本
- 支持导入/导出 JSON。

### 5.3 错误处理

- API 失败必须返回结构化错误，并保留 ASR 原文可复制。
- 失败后允许用户仅重试改写，不必重新跑 ASR。

## 6. 存储规范（冻结）

### 6.1 历史记录

历史记录必须包含：

- `task_id`
- `created_at_ms`
- `asr_text`
- `final_text`（若未启用或失败则可等于 `asr_text`）
- `template_id`（可选）
- `preprocess_ms`
- `asr_ms`
- `rtf`
- `device_used`

### 6.2 配置与模板

- 普通配置（非敏感）：可用本地配置文件（例如 JSON/TOML）。
- 模板：以独立文件或存储表保存，支持导入导出。
- 敏感配置（API Key）：必须使用 Windows 安全存储优先。

#### 6.2.1 LLM 配置项（必须可在 UI 配置）

普通配置（可落盘到 settings.json）：

- `llm_base_url`：API Base URL（例如 `https://api.openai.com/v1`）。允许用户粘贴完整 endpoint（`.../chat/completions`），应用需做归一化。
- `llm_model`：LLM 模型名（写入 Chat Completions 的 `model` 字段）。
- `llm_reasoning_effort`：推理等级（写入 Chat Completions 的 `reasoning_effort` 字段）。`default`/空表示“不发送该字段”。

敏感配置（不得明文落盘）：

- `llm_api_key`：仅存 OS Keyring（Windows 凭据管理器优先）；日志中不得出现。

加载优先级（冻结）：

1. `settings.json`（UI 保存的配置）
2. 环境变量（开发/临时覆盖）：`TYPEVOICE_LLM_BASE_URL`、`TYPEVOICE_LLM_MODEL`、`TYPEVOICE_LLM_API_KEY`
3. 内置默认值（base_url 默认 `https://api.openai.com/v1`，model 默认 `gpt-4o-mini`）

## 7. 错误模型与日志（冻结）

### 7.1 错误码（建议）

- `E_FFMPEG_NOT_FOUND`
- `E_FFMPEG_FAILED`
- `E_MODEL_NOT_INSTALLED`
- `E_MODEL_CHECKSUM_MISMATCH`
- `E_ASR_RUNNER_START_FAILED`
- `E_ASR_FAILED`
- `E_LLM_FAILED`
- `E_CANCELLED`

约束：UI 必须能将错误码映射为用户可理解文案，并提供一条建议动作。

### 7.2 日志约束

- 不记录 API Key。
- 不记录完整音频内容。
- 允许记录任务阶段、耗时、错误码与简要错误摘要。

## 8. 模型管理（冻结）

### 8.1 下载与校验（必须）

校验项至少包括：

- 文件完整性校验（hash 或 manifest 校验）
- 模型目录结构校验（必要文件存在）
- 版本标识记录（`model_id`、`model_version`、下载时间）

### 8.2 存储位置

- 允许用户选择模型存储目录。
- 默认建议放在用户数据目录（避免安装目录写权限问题）。

## 9. 分发与依赖（冻结）

- 允许安装包额外安装 VC++ Runtime。
- 允许安装包额外安装 CUDA 相关依赖。
- 内置 FFmpeg 随包分发。

## 10. 待确认/待验证

- PyTorch CUDA 的打包策略与 ASR Runner 的分发形式（独立 Python 环境、嵌入式 Python、或其他方式）。
- 模型下载源与镜像策略。
- FFmpeg 版本、构建选项与许可证声明写法。
