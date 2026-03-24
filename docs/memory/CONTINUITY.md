# CONTINUITY（外置记忆）

- 本文件只保留当前有效状态，用于在新会话中直接恢复工作。
- 不记录历史过程、日期流水、旧结论或复盘信息。
- 真源优先级：`docs/` 冻结文档 > `docs/memory/DECISIONS.md` / `SPEC.md` / `USER_PREFS.md` / `PITFALLS.md`。

## 当前有效目标

- 保持 MVP 核心链路稳定可用：录音、预处理、转录、可选改写、导出、自动粘贴、可观测。
- 保持热键路径与主 UI 路径在任务启动、上下文冻结、错误暴露上的行为一致。
- 维护 `docs/memory` 为“当前事实快照”，不保留历史堆积内容。

## 当前状态

### 已确认

- [VERIFIED] 项目当前以 Windows 桌面端为主要运行目标，Linux/WSL 主要承担开发与辅助验证。
- [VERIFIED] 任务统一入口以 `start_task` 为主，录音链路使用后端托管资产而不是前端直接持有音频字节。
- [VERIFIED] 上下文采集当前采用文本结构模型，不再走截图路径。
- [VERIFIED] 录音输入采用显式策略模型，并依赖缓存而不是在热路径里做慢速探测。
- [VERIFIED] 自动粘贴是导出阶段的一部分，要求错误可见、不可静默吞掉。
- [VERIFIED] `docs/index.md` 是文档导航入口；`docs/memory` 只保存会影响后续工作的当前事实。

### 待确认

- [UNCONFIRMED] Windows 端热键录音到转写完成的整条闭环是否仍稳定。
- [UNCONFIRMED] Windows 自动粘贴在真实目标输入框上的实机闭环是否稳定。
- [UNCONFIRMED] Linux 自动粘贴的真实桌面环境闭环是否稳定。
- [UNCONFIRMED] 热键链路与主 UI 链路在改写参数和上下文冻结上的最终一致性是否已全部补齐。

## 当前工作集

- 重点模块：
  - `apps/desktop/src-tauri/src/lib.rs`
  - `apps/desktop/src-tauri/src/pipeline.rs`
  - `apps/desktop/src-tauri/src/settings.rs`
  - `apps/desktop/src-tauri/src/task_manager.rs`
  - `apps/desktop/src-tauri/src/context_capture.rs`
  - `apps/desktop/src-tauri/src/context_capture_windows.rs`
  - `apps/desktop/src/screens/MainScreen.tsx`
  - `apps/desktop/src/screens/SettingsScreen.tsx`
- 验证入口：
  - `scripts/verify_quick.py`
  - `scripts/verify_full.py`
  - `scripts/windows/windows_gate.ps1`

## 下一步

- 优先在 Windows 端补完整闭环验证：热键录音、停止/取消、转写、改写、导出、自动粘贴。
- 若新增会影响后续工作的事实，立刻同步写回对应 memory 文件，不把状态继续堆在本文件里。
- 继续把 `CONTINUITY.md` 维持成短小、可恢复、无历史噪音的当前状态页。

## 当前约束

- 不使用静默兜底；配置缺失或关键链路异常应尽早暴露。
- 用户要求“严格按文档执行”时，只能执行文档原文命令和文档明示修复步骤。
- 代码或行为变更如果影响后续工作，必须同步更新 `docs/` 与 `docs/memory`。
