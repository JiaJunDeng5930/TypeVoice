# SPEC（外置记忆）

- 本文件是 `docs/` 目录中冻结规格的最小索引，不承载实现历史。
- 真源仍是：`docs/base-spec.md`、`docs/tech-spec.md`、`docs/verification.md`、`docs/roadmap.md`。

## 1. 目标

- 以 Windows 桌面端为主的“录完再出稿”语音输入工具：本地录音转写后可选 LLM 改写，并在导出阶段支持复制与自动粘贴。
- 目标优先级：可用性、稳定性、可取消、可观测、性能。

## 2. 范围（MVP）

- 平台：Windows 桌面端为主；自动粘贴能力需兼容 Linux、Windows 与 macOS。
- 流程：Record -> Preprocess(FFmpeg) -> Transcribe(ASR) -> Rewrite(LLM，可选) -> Persist -> Export(copy + auto paste)。
- ASR：`Qwen/Qwen3-ASR-0.6B`，PyTorch CUDA，禁止 CPU 降级。
- LLM 改写：仅在用户启用时发送转录文本与必要上下文，不上传音频。
- 历史记录：仅保存文本与元信息，不保存音频文件。

## 3. 明确不做（MVP Out of Scope）

- 流式转录、说话人分离、端上离线 LLM、复杂自动输入脚本化、全局托盘常驻（作为未来扩展）。

## 4. 验收与门禁（摘要）

- 分级验收：`quick`（<=60s）与 `full`（<=10min）。
- 核心硬约束：
  - `device_used == cuda`。
  - `cancel_latency_ms <= 300ms`。
  - `full` 验收包含 `full` 固定样本、轻压测、失败率与临时目录清理。
  - 默认不要求真实 LLM 调用；如需 `llm_smoke` 为可选校验。
- 可观测性要求：错误链路必须有稳定 `step_id` 与 `code`，并记录可定位的上下文与回溯信息。

## 5. 依赖文档

- 流程与结构由 `docs/base-spec.md`、`docs/tech-spec.md` 管理。
- 执行计划由 `docs/roadmap.md`、`docs/tasks.md` 管理。
- 验收脚本与指标由 `docs/verification.md`、`docs/fixtures-sources.md`、`docs/perf-spike.md` 管理。
