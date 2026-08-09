# TypeVoice 分级验证规格

目标：为自用工具定义轻量、可复现的验收机制。

状态：本文件记录当前 quick/full 分级。T22 已实现：quick/full 都执行显式合同计划；所有 Cargo 测试选择和 full 的前端合同都先验证非零匹配，full 明确覆盖 workspace、engine 与 frontend，主 CI 直接调用同一 counted full gate。T23 的真实 per-platform runner 已接入；当前 Linux 证据来自本地 WSL 私有 D-Bus/AT-SPI 会话中的生产 adapter 与 GTK 控件全文读回，Windows 真实输入证据只能由隔离的 hosted 或 dedicated interactive runner 产生。单平台通过或另一平台 `NotRun`/`Skipped` 都不能表述为跨平台 `PASS`。

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
- Workspace 与 engine 哨兵合同；执行前列举测试并拒绝 0-match。
- 前端构建合同。
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
- 全部 Rust 单元测试；执行前列举 workspace 测试并拒绝 0-match。
- Engine T01-T21 架构合同；显式选择并拒绝 0-match。
- 前端合同测试；要求报告非零测试数。
- 三条 fixture 的 FFmpeg 预处理验证。
- FFmpeg 预处理取消验证。

输出：

- 控制台摘要。
- 追加一条结构化记录到 `metrics/verify.jsonl`。

## 5. T23 平台插入验证

- `cargo xtask verify insertion-contract` 只执行当前 OS 对应的 ignored integration contract，Windows、Linux 与 macOS 结果分别记录；普通 workspace 单元测试不会产生原生输入。
- runner 必须设置 `TYPEVOICE_T23_ISOLATED=1`。Linux CI 还必须使用私有 Xvfb、D-Bus 与 AT-SPI registry；Windows CI 必须对 `windows-latest` 隔离 VM 另设 `TYPEVOICE_T23_WINDOWS_VM=1`；macOS runner 必须在隔离的图形会话中授予测试进程 Accessibility 权限。缺少这些条件时必须失败，不能回退到 mock 后记为 `Passed`。
- 三个平台都由独立 child 创建受控编辑控件，父测试只调用生产 `capture_insertion_target` 与 `auto_paste_text`，并以控件实际读回完整 Unicode 文本作为通过条件。单平台 `Passed` 与其他平台 `NotRun`/`Skipped` 不构成跨平台 `PASS`。

## 6. 手工验证

- 启动桌面应用。
- 选择 Doubao 或远程 HTTP ASR provider。
- 完成一次录音、转录、可选改写、复制或自动粘贴流程。
- 转录中取消一次，确认 UI 状态更新。
