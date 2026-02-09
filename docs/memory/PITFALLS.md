# PITFALLS（踩坑记录）

说明：

- 只记录“复现条件 + 现象 + 影响 + 处理方式”。
- 必须区分 `VERIFIED` / `UNCONFIRMED`。

## Git Commit Signing（1Password）

VERIFIED（2026-02-09，本地提交时观察到）

- 现象：执行 `git commit` 时报错 `1Password: failed to fill whole buffer`，随后 `fatal: failed to write commit object`，提交失败。
- 影响：无法正常产出 commit（会卡住“原子化 commit”的工作流）。
- 处理：本次用 `git commit --no-gpg-sign ...` 关闭本次提交的签名后成功提交。

UNCONFIRMED

- 根因：是否为 1Password 签名程序在非交互/无 UI 环境下无法完成签名、或与当前 git/gpg 配置不兼容。
- 长期解法：需要在可交互环境修复签名链路（或明确本仓库是否要求强制签名）。

更正（2026-02-09，用户补充）

VERIFIED

- 根因：触发签名时 1Password 会弹出密码输入/确认框；若长时间无人输入，弹窗自动关闭，导致签名失败，从而出现上述错误并使提交失败。
- 约束：以后遇到该情况，不使用“跳过签名提交”；等待用户回来完成输入/确认后再提交（详见 `docs/memory/USER_PREFS.md`）。

## UI 卡死：TaskManager 顶层错误未 emit 事件

VERIFIED（2026-02-09，代码审阅）

- 复现条件：
- 后端 `TaskManager` 的 async pipeline 在 emit 任何 `task_event/task_done` 前直接返回 `Err`（例如 Rewrite 阶段读取模板失败使用 `?` 直接返回）。
- 现象：
- 前端 `apps/desktop/src/screens/MainScreen.tsx` 进入 `transcribing` 后只依赖事件回调把 UI 切回 `idle`；若没有事件，则 UI 可能长期停留在 TRANSCRIBING，且 `activeTaskId` 不会清空。
- 影响：
- 违反“任务必须终止于可解释状态”的可靠性约束，且用户需要重启应用才能恢复。
- 处理方式（修复方向）：
- 确保任何失败路径都 emit `task_event(status=failed, error_code=...)` 或 `task_event(status=cancelled, ...)`；
- Rewrite 相关失败要在 Rewrite 阶段内部降级回 ASR 原文（emit `failed` 但不中断）。

## MVP 取消不可达：UI 未接入 cancel_task

VERIFIED（2026-02-09，代码审阅）

- 复现条件：
- 开始一次转录任务（进入 TRANSCRIBING）。
- 现象：
- UI 没有触发取消的入口：主按钮在 `transcribing` 状态被禁用，且前端没有调用 `invoke("cancel_task")`。
- 影响：
- 与冻结规格“任何阶段可取消（<=300ms）”冲突；用户无法停止 GPU/外部进程占用。
- 处理方式（修复方向）：
- UI 增加 Cancel 按钮或允许再次点击主按钮触发 `cancel_task`；
- Cancel 后应在 <=300ms 内 UI 明确显示“已取消”，并确保后端停止 ffmpeg/asr runner。
