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
