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

## 统一行尾为 LF（仅源码/配置范围）

VERIFIED（2026-02-09）

- 背景：本仓库采用“WSL 统一编辑、Windows 侧仅用于编译运行验证”的双工作副本模式；若 Windows checkout 发生 CRLF 漂移，容易导致大量文件被误判为变更，并放大增量编译失效的概率（见 `docs/windows-dev-from-wsl-v0.1.md` 中对 Windows 侧编辑与行尾警告的提醒）。
- 决策：新增根目录 `.gitattributes`，对源码/常见配置与脚本文件类型强制 `eol=lf`，避免行尾漂移造成噪声改动与无效重编译。
- 方案：`.gitattributes` 覆盖 `*.rs/*.ts(x)/*.py/*.toml/*.json/*.md/*.ps1/*.sh` 等；并显式约束 `Cargo.lock` 为 LF。
- 复核方式：
  - 在 Windows repo `git pull` 后执行 `git status`，不应出现大面积“仅行尾变化”的改动；
  - 在 WSL repo `git status` 保持干净。

## 减少 dev profile 调试信息以缩短链接时间

VERIFIED（2026-02-09）

- 背景：Windows 侧 `tauri dev` 会频繁触发 Rust debug 构建；在依赖体量较大的情况下（`tauri`/`tokio`/`reqwest`/`rusqlite` 等，见 `apps/desktop/src-tauri/Cargo.toml`），链接阶段往往成为迭代瓶颈。
- 决策：在 `apps/desktop/src-tauri/Cargo.toml` 中为 dev profile 设置 `debug = 1`，减少调试信息生成量，以降低链接开销并加快增量迭代。
- 取舍：调试符号更少，但仍保留基本可用的回溯与调试能力；不影响 release 构建与运行时行为。
- 复核方式：
  - 在 Windows repo `apps/desktop/src-tauri/` 目录下，对比改动前后 `Measure-Command { cargo build }` 的耗时（尤其是增量编译 + 链接）。

## Windows Gate 可选启用 sccache（Rust 编译缓存）

VERIFIED（2026-02-09）

- 背景：Windows 侧仅用于编译/运行验证，Rust 依赖体量较大时，重复编译会明显拖慢迭代；同时不希望把额外工具作为硬依赖阻塞新环境启动（见 `docs/windows-gate-v0.1.md` 的“一键 gate”目标）。
- 决策：在 `scripts/windows/windows_gate.ps1` 中检测 `sccache`，若存在则自动设置 `RUSTC_WRAPPER=sccache` 并使用 repo-local `SCCACHE_DIR`；未安装时仅提示安装方式并继续执行。
- 取舍：将加速能力作为“可选增强”，避免因缺少 `sccache` 影响现有 Windows gate。
- 复核方式：
  - Windows 上执行 `scripts/windows/windows_gate.ps1`，应能看到 `sccache enabled` 或 `sccache not found (optional)` 的 INFO 输出；
  - 若启用后，运行 `sccache --show-stats` 观察 cache hits 增长。
