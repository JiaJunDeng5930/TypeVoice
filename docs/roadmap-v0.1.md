# TypeVoice 里程碑与 Gate v0.1（Windows MVP）

原则：

- 里程碑以“可用切片（vertical slice）+ 风险优先（risk-first）”推进。
- 每个里程碑都有明确 Gate；Gate 绑定 `docs/verification-v0.1.md` 的 `quick/full`。
- 自用工具：Gate 输出只需要控制台摘要 + 机器可读指标（JSONL），不做长报告。

## M0：规格冻结（已完成）

完成条件：

- `docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`、`docs/verification-v0.1.md` 已冻结
- fixtures 本机可用且来源记录完整（`docs/fixtures-sources-v0.1.md`）

Gate：

- 人工确认：文档一致性通过

## M1：ASR Runner 垂直切片（无 UI，风险优先）

目标：

- 在 Windows + RTX 4060 Laptop 环境跑通本地 ASR（PyTorch CUDA），并满足 RTF “必须达标”。
- 建立 `quick/full` 的最小验证入口（不依赖录音与 UI）。

交付物（工程层面）：

- ASR Runner（独立进程）可输入音频路径输出 JSON（text + metrics）
- 一个验证入口（CLI/脚本/内部命令均可）能跑 fixtures 并输出 PASS/FAIL + JSONL 指标

Gate：

- `quick`：
  - `zh_10s`：text 非空 + `device_used=cuda` + 输出包含 rtf
  - 取消：转录中取消 <=300ms 且计算停止
- `full`：
  - `zh_60s` 与 `zh_5m`：RTF 达到 `docs/base-spec-v0.1.md` 的必须阈值
  - 任一用例若 `device_used!=cuda` 直接 FAIL（不允许 CPU 降级）

## M2：FFmpeg 预处理切片（与 ASR 串联）

目标：

- 内置 FFmpeg 被应用正确定位与调用
- fixtures 全部先预处理再喂给 ASR（统一输入格式）

Gate：

- `quick`：
  - `zh_10s`：预处理成功 + 产物存在 + ASR 仍达标
- `full`：
  - 三条样本均走“预处理->ASR”，RTF 仍达标
  - 取消覆盖：预处理阶段取消一次（<=300ms 且 ffmpeg 进程停止）

## M3：最小桌面应用壳（Tauri + UI，先可用再好看）

目标：

- UI 能触发“用 fixtures 跑一次转录”，并显示阶段与结果
- 指标与错误在 UI 可见（至少最近一次任务）

Gate：

- `quick`：仍能在无 UI 场景跑通（防止 UI 影响核心）
- 手工 Gate（<=2 分钟）：
  - 在 UI 点按钮跑 `zh_10s`，能看到阶段流转与最终文本
  - 一键复制可用

## M4：录音（端到端自用 MVP）

目标：

- 完整流程：录音 -> 预处理 -> ASR -> 展示 -> 复制

Gate：

- `quick/full`：fixtures 仍必须通过（防回归）
- 手工 Gate：
  - 录 10 秒中文，能出稿并复制
  - 取消覆盖：转录中取消一次

## M5：LLM 改写 + 模板系统（UI 可编辑）

目标：

- 默认模板：纠错、表达澄清
- 模板可在 UI 编辑并立即生效
- LLM 失败回退到 ASR 原文

Gate：

- `quick/full`：默认不跑真实 API（仍需 fixtures 全通过）
- 手工 Gate：
  - 修改模板后下一次改写输出可见变化
- 可选 Gate（`llm_smoke`）：
  - 跑一次真实 API 连通性验证

## M6：历史记录与设置（自用体验完善）

目标：

- 历史仅保存文本与必要元信息
- Key 安全存储（不明文落盘、不出现在日志）

Gate：

- `full`：增加“日志/配置检索”检查不含 API Key 明文

## 未来里程碑（非 MVP）

- 热键 / 托盘 / 自动输入：在 MVP 稳定后再做（会引入平台 API 适配与更多权限/兼容性问题）

