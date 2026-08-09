# docs 索引

## 核心规格与约束

- [base-spec.md](./base-spec.md)：产品目标、范围、性能约束、验收阈值（含自动粘贴约束）。
- [tech-spec.md](./tech-spec.md)：目标线协议、能力边界、单次运行资源以及数据、错误和观测约束；业务状态以 architecture 为唯一来源。
- [verification.md](./verification.md)：当前 quick/full 级别、T22 非零合同选择与 T23 隔离的真实 per-platform runner；单平台证据不构成跨平台 `PASS`。
- [fixtures-sources.md](./fixtures-sources.md)：验证音频样本来源与命名约定。

## 架构与计划

- [architecture.md](./architecture.md)：已实现的唯一状态所有者、四态转移、取消、自动化验收合同与平台证明边界。
- [roadmap.md](./roadmap.md)：里程碑与 Gate（含复制+自动粘贴闭环）。
- [tasks.md](./tasks.md)：按里程碑分解的可执行任务清单。
- [perf-spike.md](./perf-spike.md)：性能指标、样本策略与优化输出。

## 运维与实验

- [windows-dev.md](./windows-dev.md)：从 WSL 到 Windows 的开发流程与常见操作顺序。
- [windows-gate.md](./windows-gate.md)：Windows 一键门禁与快速编译检查。
- [macos-dev.md](./macos-dev.md)：macOS Apple Silicon 的受控工具链、开发启动与系统权限。
- [repository-automation.md](./repository-automation.md)：仓库自动化策略（含 Dependabot 限流与分组规则）。
- [llm-prompt-lab.md](./llm-prompt-lab.md)：LLM 提示词实验脚本与调参流程。
- [privacy-data-flow.md](./privacy-data-flow.md)：本地/远程 ASR 与 LLM 改写的数据流与隐私边界。
- [release-process.md](./release-process.md)：版本发布、changelog 与 release notes 流程。
