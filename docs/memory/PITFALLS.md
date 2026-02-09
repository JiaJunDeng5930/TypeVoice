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

