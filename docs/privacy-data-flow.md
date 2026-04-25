# 隐私与数据流

本文档描述 TypeVoice 在不同模式下的数据流与用户可控开关。

## 1. Doubao 流式 ASR

- 当 `asr_provider=doubao` 时，录音音频会发送到 Doubao ASR 服务。
- 历史记录仅保存文本和元信息，不保存音频文件。

## 2. 远程 ASR 模式

- 当 `asr_provider=remote` 时，音频会发送到用户配置的远程 ASR 服务。
- 远程地址由设置项 `remote_asr_url` 控制。

## 3. LLM 改写（可选）

- 仅在用户启用改写时，发送文本与必要上下文到 LLM API。
- 不发送原始音频。
- API Key 通过 keyring 保存，不写入日志。

## 4. 如何关闭网络相关能力

- 关闭 LLM 改写：在设置中关闭 rewrite 开关。

## 5. 日志最小化原则

- 不记录 API Key。
- 不记录完整音频内容。
- 错误链路记录 `task_id`、错误码和必要诊断摘要。
