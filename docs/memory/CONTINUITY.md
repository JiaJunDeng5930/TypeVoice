# CONTINUITY（外置记忆）

- 本文件记录当前可执行进度与工作集，必须可直接恢复下一步工作，不包含历史复盘。
- 优先以 `docs/` 中冻结文档为真源，`docs/memory` 仅保留当前状态快照。

## 当前有效目标

- 保持 MVP 核心能力的稳定可用性：录音、预处理、转录、可选改写、复制、可观测。
- 继续压实可复用的上下文与错误诊断链路，支持热键与 UI 两条路径行为一致。

## 当前状态（进行中）

### 已确认

- 结构化诊断链路已启用：`TYPEVOICE_DATA_DIR/trace.jsonl` 为主日志入口，关键路径记录稳定的 `step_id` 与 `code`。
- 上下文采集与改写参数链路按设置主源，支持配置缺失即失败（fail-fast），不引入静默兜底。
- `docs/` 文档索引与命名清洗已完成，历史版本后缀已移除。
- 可复用的执行索引文件已新增：`docs/index.md`。
- 词汇表重写入口已完成：新增 `rewrite_glossary` 配置持久化与 rewrite content 透传，默认提示词已加入 `### GLOSSARY` 约束说明。

### 当前工作集

- 需要在下一步优先验证：
  - 热键触发链路的 rewrite 一致性（`rewrite_enabled/template_id` 与设置一致）。
  - 关键 Windows-only 改动后的本机 `Windows` `npm run tauri dev` 与 `quick/full` 可回归。
- 进行中更新点：`docs/memory` 由复盘内容转为“当前事实清单”。
- ASR 设置页状态来源修正：状态检查已改为按 `settings` 解析后的实际 ASR 模型来源（本地路径或远端模型 id）执行，不再固定指向仓库默认目录。
- 完整性告警策略调整：`manifest.json_missing` 仍可见，但不再阻断 ASR 可用性，状态面板会明确标注为“可用但未做完整性清单校验”。

## 下一步建议

- 在 Windows 会话里补齐一次“热键录音 -> 转写 -> 改写 -> persist -> copy”闭环，并将观察结果回写：
  - 正常：同一 `task_id` 链路内 `CMD.start...`、`TASK.rewrite_effective`、`task_done` 一致。
  - 异常：将复现条件与处理思路写入 `docs/memory/PITFALLS.md`。
- 在本仓执行 `git status` 与 `./.venv/bin/python scripts/verify_full.py` 前后对照文档约定。
- 避免改动 `docs/memory` 以外文档时引入重复事实，若新增约束立即写回对应文件。

## 变更后固定执行流程

- 所有代码修复/功能更新完成后，Windows 侧必须按以下顺序执行：
  1. 同步源码（拉取当前分支最新提交）。
  2. 重新编译（推荐按项目文档链路执行）。
  3. 以 Debug 模式启动进程并复测相关链路。

## 关键文件（当前可直接核验）

- `apps/desktop/src-tauri/src/trace.rs`：trace/diagnostic 主流程。
- `apps/desktop/src-tauri/src/context_capture.rs`、`apps/desktop/src-tauri/src/context_capture_windows.rs`：上下文采集。
- `apps/desktop/src-tauri/src/lib.rs`、`pipeline.rs`、`settings.rs`、`templates.rs`：流程入口与配置。
- `scripts/verify_quick.py`、`scripts/verify_full.py`：验收闭环。
- `docs/index.md`：文档导航入口。

## 关键命令（仅当前可复用）

- `./.venv/bin/python scripts/verify_quick.py`
- `./.venv/bin/python scripts/verify_full.py`
- Windows 环境文档命令：
  - `Set-Location <repo>/apps/desktop`
  - `npm run tauri dev`
- WSL 下触发 Windows 文档命令必须保持文档命令语义不变，仅调整调用链环境。

## 关键约束

- 不增加用户交互类“发送预览/确认”流程；上下文采集与发送保持自动化。
- 所有新增行为变更须同时落在 `docs/*` 与对应 `docs/memory` 文件里。
- 任何多路径并发日志结论必须按 `task_id` 聚合，不按尾部单行推断可用性。
