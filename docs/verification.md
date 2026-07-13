# TypeVoice 分级验证规格

目标：为自用工具定义轻量、可复现的验收机制。

状态：本文件记录当前 quick/full 分级；现有实现尚未证明测试选择器执行了非零 workspace 状态合同，也没有分别执行 Windows platform input 与 Linux AT-SPI。目标补充 gate 见 `architecture.md` T22/T23，在实现前不得把一次手工自动粘贴或单一 OS 的 Skipped 结果表述为跨平台 PASS。

## 1. 分级与时间预算

- `quick`：快速验证，单次 <= 60 秒，用于每个新 commit 前后。
- `full`：全量验证，单次 <= 10 分钟，用于多日累积改动后的整体确认。

## 2. 通用度量对象

- 固定样本音频：`fixtures/` 下的 `zh_10s.ogg`、`zh_60s.ogg`、`zh_5m.ogg`。
  - 音频本体不提交到 git。
  - 下载地址与 `sha256` 固化在 `scripts/fixtures_manifest.json`。
  - `cargo xtask verify quick/full` 会在运行前自动下载并校验缺失样本。
- 结构化指标：每次验证输出关键指标、成功/失败与错误码。

## 3. `quick`

必须包含：

- Rust 后端编译检查。
- 可调试性契约检查。
- FFmpeg 预处理参数契约检查。
- FFmpeg 预处理取消验证。

输出：

- 控制台一行摘要：PASS/FAIL + `cancel_ffmpeg_ms`。
- 追加一条结构化记录到 `metrics/verify.jsonl`。

## 4. `full`

必须包含：

- Rust 后端编译检查。
- 可调试性契约检查。
- 全部 Rust 单元测试。
- 三条 fixture 的 FFmpeg 预处理验证。
- FFmpeg 预处理取消验证。

输出：

- 控制台摘要。
- 追加一条结构化记录到 `metrics/verify.jsonl`。

## 5. 手工验证

- 启动桌面应用。
- 选择 Doubao 或远程 HTTP ASR provider。
- 完成一次录音、转录、可选改写、复制或自动粘贴流程。
- 转录中取消一次，确认 UI 状态更新。
