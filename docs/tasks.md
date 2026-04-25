# TypeVoice 任务拆解（Windows MVP）

说明：

- 本任务列表以 `docs/roadmap.md` 为里程碑蓝图。
- 每个任务都要能支撑 `docs/verification.md` 的 `quick/full` Gate。
- 当前 ASR provider 为 Doubao 流式 ASR 与远程 HTTP ASR。

---

- [x] 1. FFmpeg 工具链与预处理
  - 内置 FFmpeg 定位与校验。
  - 预处理输出格式：WAV/PCM/16k/mono。
  - quick/full 覆盖预处理与取消。

- [x] 2. 桌面壳与录音链路
  - Tauri + React UI。
  - 录音开始、停止、取消。
  - 阶段状态、耗时与错误码展示。

- [x] 3. ASR provider
  - Doubao 流式 ASR。
  - 远程 HTTP ASR。
  - API Key / 凭据通过 keyring 或环境变量读取。

- [x] 4. LLM 改写与模板系统
  - 模板可在 UI 编辑并立即生效。
  - LLM 失败时保留 ASR 原文。

- [x] 5. 历史记录与导出
  - 历史记录只保存文本与元信息。
  - 支持复制与自动粘贴。

- [x] 6. 验证体系
  - `typevoice-tools verify quick`：编译、关键单测、FFmpeg 参数契约、FFmpeg 取消验证。
  - `typevoice-tools verify full`：编译、完整 Rust 单测、fixtures 预处理验证。
