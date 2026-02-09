# DECISIONS（关键决策与理由）

说明：

- 记录“做了什么关键取舍/为什么”，用于处理冲突与版本漂移。
- 必须区分 `VERIFIED` / `UNCONFIRMED`。

## Windows Release 启动栈溢出：提升主线程栈保留大小

VERIFIED（2026-02-09）

- 背景：Windows Release 版 `typevoice-desktop.exe` 启动后闪退，事件日志显示异常 `0xc00000fd`（STATUS_STACK_OVERFLOW），且复现稳定。
- 决策：为 Windows MSVC 目标设置更大的主线程栈保留大小（8MB），避免启动阶段栈深导致的溢出。
- 方案：在 `apps/desktop/src-tauri/.cargo/config.toml` 添加：
- `rustflags = ["-C", "link-arg=/STACK:8388608"]`
- 取舍：属于“运行时健壮性优先”的工程化修复；不改变业务逻辑，仅改变链接器栈配置。
- 复核方式：检查 PE 头 `SizeOfStackReserve` 为 `0x00800000`；并验证 release 进程稳定运行 >= 30s 且无新的 APPCRASH 事件。

