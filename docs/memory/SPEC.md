# SPEC（外置记忆）

说明：

- 本仓库“冻结规格”的真源在 `docs/`（例如 `docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`、`docs/verification-v0.1.md`、`docs/roadmap-v0.1.md`）。
- 本文件只维护“最小可用”的目标/范围/验收提要与显式未知项，避免重复抄写造成漂移。
- 本文件内容必须标注 `VERIFIED` / `UNCONFIRMED`。若与冻结文档冲突，以冻结文档为准（VERIFIED：见 `README.md`）。

## 1. 目标

VERIFIED

- 面向中文输入场景的 Windows 桌面“录完再出稿”语音打字工具：录音结束后本地 ASR 转录，再可选调用 LLM API 改写，最后一键复制文本。来源：`README.md`、`docs/base-spec-v0.1.md`。
- MVP 优先：可用性、稳定性、速度、可取消、可观测（阶段与耗时可见）。来源：`docs/base-spec-v0.1.md`。

UNCONFIRMED

- 是否需要在 `SPEC.md` 内写入更细的“验收检查表”全文，或仅保留索引与最小摘要（当前采取“最小摘要 + 指向冻结文档”策略）。

## 2. 范围（MVP）

VERIFIED

- 平台：Windows 桌面端（MVP 仅 Windows）。来源：`docs/base-spec-v0.1.md`。
- 交互：非流式（不是边说边出字），录完再处理。来源：`docs/base-spec-v0.1.md`。
- 核心流水线阶段：Record -> Preprocess(FFmpeg) -> Transcribe(ASR) -> Rewrite(LLM，可选) -> Persist(历史文本) -> Export(复制)。来源：`docs/tech-spec-v0.1.md`。
- 本地 ASR：模型 `Qwen/Qwen3-ASR-0.6B`；推理后端 PyTorch CUDA；不允许 CPU 降级。来源：`docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`、`docs/verification-v0.1.md`。
- LLM 改写：仅在启用时上传“转录文本 + 相关上下文”（例如剪贴板/最近历史/上一外部窗口截图等），不上传音频；失败必须回退保留 ASR 原文可复制；配置需可在 UI 内设置。来源：`docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`。
- 历史记录：仅保存文本与元信息，不保存音频。来源：`docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`。

## 3. 明确不做（MVP Out Of Scope）

VERIFIED

- 流式转录、说话人分离、端上离线 LLM、自动输入/热键/托盘常驻。来源：`docs/base-spec-v0.1.md`。

## 4. 验收与 Gate

VERIFIED

- 验证分级：`quick`（<=60s）与 `full`（<=10min）。来源：`docs/verification-v0.1.md`。
- 关键硬约束：`device_used == cuda`；取消 <= 300ms；`full` 覆盖 RTF 阈值与轻压测；默认不跑真实 LLM。来源：`docs/verification-v0.1.md`、`docs/base-spec-v0.1.md`。
- 里程碑与 Gate：见 `docs/roadmap-v0.1.md`、`docs/tasks-v0.1.md`。

UNCONFIRMED

- Windows 原生验收 Gate（M12）当前是否已在目标 Windows 机器上跑通并记录结果（`docs/tasks-v0.1.md` 里仍未勾选）。

## 5. 待确认/待验证清单（从冻结文档抽取）

UNCONFIRMED

- 模型下载源与镜像策略。来源：`docs/base-spec-v0.1.md`、`docs/tech-spec-v0.1.md`。
- 模型版本升级策略（多版本共存/回滚）。来源：`docs/base-spec-v0.1.md`。
- FFmpeg 版本、构建选项与许可证声明写法。来源：`docs/tech-spec-v0.1.md`。
- PyTorch CUDA 打包策略与 ASR Runner 分发形式。来源：`docs/tech-spec-v0.1.md`。
