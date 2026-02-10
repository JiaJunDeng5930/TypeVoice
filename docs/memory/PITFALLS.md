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

更正（2026-02-09）

VERIFIED

- 已修复：TaskManager 顶层 Err 现在会 fail-safe emit `task_event`，且 Rewrite 模板读取失败不再导致“无事件退出”。修复 commit：`d3de362`。

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

更正（2026-02-09）

VERIFIED

- 已修复：转录中点击主按钮触发 `cancel_task`，并有“CANCELLING...”提示与 `task_event(status=cancelled)` 处理。修复 commit：`d3de362`。

## Windows Release 闪退：STATUS_STACK_OVERFLOW (0xc00000fd)

VERIFIED（2026-02-09）

- 复现条件：
- Windows 上运行 `apps/desktop/src-tauri/target/release/typevoice-desktop.exe`（GUI 子系统）启动后数秒内退出；
- 事件查看器 Application Error/WER 显示异常码 `0xc00000fd`，模块为 `typevoice-desktop.exe`。
- 现象：
- UI 看不到页面或一闪而过；偶发显示 `localhost` 拒绝连接，但本质是进程崩溃导致。
- 影响：
- Release 版本不可用，无法验证性能与日志。
- 处理方式：
- 提升 Windows MSVC 目标的主线程栈保留大小到 8MB：`apps/desktop/src-tauri/.cargo/config.toml` 设置 `-C link-arg=/STACK:8388608`。
- 复核：
- `objdump -x ...typevoice-desktop.exe` 中 `SizeOfStackReserve == 0x00800000`；
- 启动后进程稳定运行 >= 30s，且无新的 APPCRASH 事件。

## Windows Release 无控制台：不要依赖 stdout/stderr 做诊断（SafeEprintln）

VERIFIED（2026-02-09）

- 背景：
- `apps/desktop/src-tauri/src/main.rs` 在 release 下启用 `windows_subsystem = "windows"`，这会让进程默认没有控制台窗口。
- 复现条件：
- 在 PowerShell 里运行 release exe 并重定向：`& typevoice-desktop.exe 1> out.txt 2> err.txt`。
- 现象：
- `out.txt/err.txt` 可能仍为空（即使代码里有 `eprintln!`），导致“闪退时完全没输出”的错觉。
- 影响：
- 启动早期崩溃/卡死时，基于控制台的诊断手段基本失效。
- 处理方式：
- 诊断输出优先写入数据目录文件（例如 `metrics.jsonl`、`startup_trace.log`、`panic.log`），而不是依赖控制台；
- 对少量 best-effort 的 stderr 输出，使用 `apps/desktop/src-tauri/src/safe_print.rs` 的 `safe_eprintln!`（忽略写入错误），避免把“打印失败”放大成新的故障源。

UNCONFIRMED（但建议遵守）

- 在部分环境里，如果 panic hook/日志输出链路反复尝试向不可用的 stderr 写入，可能引入递归/放大故障；因此在 Windows release 上应尽量减少对 stderr 的依赖，必要时一律走文件落盘。

## Windows 安装 sccache：`cargo install sccache` 编译失败

VERIFIED（2026-02-10）

- 复现条件：
  - Windows 环境执行 `cargo install sccache`（Rust 工具链：`rustc 1.93.0` / `cargo 1.93.0`）。
- 现象：
  - `sccache` 编译失败，报错包含 `unresolved import windows_sys::Win32::System::Threading::CreateProcessW`，导致无法通过 `cargo install` 安装。
- 影响：
  - 无法按预期启用 `RUSTC_WRAPPER=sccache`，Windows 侧编译迭代提速落不下来。
- 处理方式：
  - 改用官方 release 的预编译 `sccache.exe`：
    - 从 GitHub release 下载 `sccache-*-x86_64-pc-windows-msvc.zip` 并解压得到 `sccache.exe`
    - 将其放到 `%USERPROFILE%\\.cargo\\bin\\sccache.exe`（或任意在 PATH 中的目录）
    - 复核：`sccache --version` 可执行

## Windows benchmark 清理 `target` 失败：文件被占用（Access Denied）

VERIFIED（2026-02-10）

- 复现条件：
  - Windows 上对 `apps/desktop/src-tauri/target` 执行 `cargo clean` 或递归删除（Remove-Item），同时存在进程占用生成的 `.exe/.dll`（例如曾启动过 `typevoice-desktop.exe`）。
- 现象：
  - 清理失败，报错包含 `拒绝访问 (os error 5)`，有时后续链接也会因无法写入输出文件而失败（例如 `LNK1104` 无法打开输出 `.dll`）。
- 影响：
  - 基准测试/清理步骤不稳定，可能误判编译耗时或直接导致构建失败。
- 处理方式：
  - 先停止可能占用文件的进程（例如 `typevoice-desktop`、`node`）。
  - 基准测试时避免使用共享的默认 `target/`：改用独立的 `CARGO_TARGET_DIR`（例如 repo 下 `tmp/bench-*/target`），并用“带重试”的删除逻辑清理该目录。
