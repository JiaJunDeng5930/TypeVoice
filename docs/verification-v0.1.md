# TypeVoice 分级验证规格 v0.1

目标：为“自用工具”定义轻量、可复现的验收机制。发布合规、审计留档、面向人类的详细报告均不在本规格范围内。

## 1. 分级与时间预算（冻结）

- `quick`：快速验证，单次 <= 60 秒，用于每个新 commit 前后。
- `full`：全量验证，单次 <= 10 分钟，用于多日累积改动后的整体确认。

## 2. 通用度量对象（冻结）

- 固定样本音频：本机 `fixtures/` 下的 `zh_10s.ogg`、`zh_60s.ogg`、`zh_5m.ogg`（来源与时长见 `docs/fixtures-sources-v0.1.md`；音频不提交到 git）。
- 结构化指标：每次验证输出关键指标（RTF、device_used、cancel_latency_ms、成功/失败与错误码）。只需要机器可读记录 + 控制台摘要，不需要长报告。

## 3. 关键约束（冻结）

- 不允许降级到 CPU：验证中若 `device_used != cuda`，视为失败（除非显式进入“诊断模式”，诊断模式不作为 Gate）。
- `full` 验证默认不调用真实 LLM API（避免网络波动与费用），但允许提供可选 `llm_smoke` 开关做一次真实连通性检查。

## 4. `quick`（<=60s）

目标：尽快发现“明显坏了”的问题（跑偏到 CPU、输出为空、取消失效、模板系统崩）。

必须包含：

- 单元测试（仅纯逻辑、无 GPU/无网络部分）
- ASR 烟雾测试（`fixtures/zh_10s.ogg`）
- 断言：`text` 非空
- 断言：`device_used == cuda`
- 断言：输出包含 `rtf` 指标
- 取消烟雾测试（在转录阶段触发一次取消）
- 断言：`cancel_latency_ms <= 300`
- 断言：转录计算实际停止（不继续占用 GPU）

输出：

- 控制台一行摘要：PASS/FAIL + `rtf` + `device_used` + `cancel_latency_ms`
- 追加一条结构化记录（JSONL 或等价格式）

## 5. `full`（<=10min）

目标：覆盖性能阈值、取消、资源释放与常见失败路径，确保自用体验稳定。

必须包含：

- 全部单元测试
- ASR 性能套件（`fixtures/zh_10s.ogg`、`fixtures/zh_60s.ogg`、`fixtures/zh_5m.ogg`）
- 断言：`device_used == cuda`（任一用例跑到 CPU 视为失败）
- 断言：RTF 达到基础规格阈值（见 `docs/base-spec-v0.1.md`）
- 取消覆盖：至少覆盖预处理阶段与转录阶段各一次
- 断言：`cancel_latency_ms <= 300` 且计算停止
- 稳定性轻压测（时间盒）：建议 3 分钟循环转录 `zh_10s.ogg`
- 断言：0 crash、0 hang、失败率为 0
- 断言：临时目录无堆积（任务结束后应清理）
- 隐私/安全轻检查
- 断言：默认不保存音频（任务后无音频残留）
- 断言：日志与配置中不出现 API Key 明文（简单检索规则即可）

可选项（不默认启用）：

- `llm_smoke`：真实调用一次 LLM API，只验证连通性与失败回退（失败时仍能复制 ASR 原文）。
  - 前置条件：已在 UI 内配置 LLM API Base URL / Model / 推理等级（可选）并设置 API Key（Keyring）。
  - 验收点：HTTP 2xx 且输出非空；若返回 4xx/5xx 也必须以 `E_LLM_FAILED` 失败并保留 ASR 原文可复制。

输出：

- 控制台摘要 + 结构化记录（同 `quick`）
