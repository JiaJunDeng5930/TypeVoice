# TypeVoice

面向中文输入场景的 Windows 桌面“录完再出稿”语音打字工具。

本仓库当前阶段以“规格冻结”为主：只有写入文档的内容才视为可靠的工程约束与验收依据。

## 文档

- `docs/index.md`：文档导航与摘要索引。
- `docs/base-spec.md`：基础规格（业务需求 + 质量保证 + 性能要求 + 验收标准）
- `docs/tech-spec.md`：技术规格（模块边界、接口、数据与错误模型、分发与依赖策略）
- `docs/perf-spike.md`：性能验证计划（RTF 指标、样本集、测量口径、通过标准）
- `docs/verification.md`：分级验证（quick/full）的度量、保证与验收口径
- `docs/fixtures-sources.md`：本机 fixtures 音频来源与命名约定（音频本身不提交）
- `docs/architecture.md`：架构边界与接口（为 Gate 反推的工程结构）
- `docs/roadmap.md`：里程碑与 Gate（每个里程碑的验收条件与验证方式）
- `docs/tasks.md`：任务拆解（按里程碑的可执行 checklist，直接绑定 quick/full Gate）
- `docs/windows-gate.md`：Windows 原生验收 Gate（PowerShell 一键脚本）
- `docs/llm-prompt-lab.md`：LLM Prompt 调参脚本（请求/响应落盘，人工判断输出）
