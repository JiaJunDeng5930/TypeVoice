# TypeVoice 里程碑与 Gate（Windows MVP）

原则：

- 里程碑以可用切片和风险优先推进。
- 每个里程碑都有明确 Gate；Gate 绑定 `docs/verification.md` 的 `quick/full`。
- Gate 输出控制台摘要与机器可读指标（JSONL）。

## M0：规格冻结

完成条件：

- `docs/base-spec.md`、`docs/tech-spec.md`、`docs/verification.md` 保持一致。
- fixtures 本机可用且来源记录完整（`docs/fixtures-sources.md`）。

## M1：FFmpeg 工具链

目标：

- 内置 FFmpeg 被应用正确定位与调用。
- fixtures 可统一预处理为 WAV/PCM/16k/mono。

Gate：

- `quick`：预处理取消 <=300ms。
- `full`：三条样本均可完成预处理。

## M2：桌面应用壳

目标：

- UI 能展示录音、转录、改写、插入阶段。
- 指标与错误在 UI 可见。

Gate：

- `quick`：无 UI 场景通过。
- 手工 Gate：启动桌面应用并完成一次录音流程。

## M3：ASR provider

目标：

- 支持 Doubao 流式 ASR。
- 支持远程 HTTP ASR。
- 凭据与 API Key 通过设置页管理。

Gate：

- 手工 Gate：Doubao 或远程 HTTP provider 能产出文本。
- 取消 Gate：转录中取消能更新状态并释放资源。

## M4：LLM 改写 + 模板系统

目标：

- 默认模板：纠错、表达澄清。
- 模板可在 UI 编辑并立即生效。
- LLM 失败时保留 ASR 原文。

Gate：

- `quick/full`：默认跳过真实 LLM API。
- 手工 Gate：修改模板后下一次改写输出可见变化。

## M5：历史记录与设置

目标：

- 历史记录仅保存文本与元信息。
- 设置页覆盖 ASR、LLM、录音输入、热键、导出等配置。

Gate：

- 清空历史后 UI 与数据库一致。
- 修改设置后下一次任务使用新配置。
