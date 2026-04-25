# 仓库自动化

## CI

- 配置文件：`.github/workflows/ci.yml`
- `rust` job 运行在 `ubuntu-latest`，在执行 `cargo check --locked` 之前必须先安装 Tauri Linux 构建依赖：
  - `libglib2.0-dev`
  - `libgtk-3-dev`
  - `libwebkit2gtk-4.1-dev`
  - `libayatana-appindicator3-dev`
  - `librsvg2-dev`
  - `patchelf`
- `rust` job 还必须先在 `apps/desktop` 执行 `npm ci` 和 `npm run build`，确保 Tauri 配置中的 `build.frontendDist=../dist` 已生成；否则 `tauri::generate_context!()` 会在编译期因找不到前端产物而失败。
- 原因：Tauri 的 Linux 依赖通过 `pkg-config` 解析；若缺失这些系统包，CI 会在 `glib-sys` / `gio-sys` / `gobject-sys` 等 crate 的 build script 阶段失败，而不是进入业务代码编译。

## Dependabot

- 配置文件：`.github/dependabot.yml`
- 监控范围：
  - `npm`：`/apps/desktop`
  - `cargo`：`/apps/desktop/src-tauri`
  - `cargo`：`/tools/typevoice-tools`

## 当前策略

- 每个生态继续按 `weekly` 检查依赖更新。
- `version-updates`：
  - 每个生态同时最多保留 `1` 个打开中的版本更新 PR（`open-pull-requests-limit: 1`）。
  - 同一生态的版本更新按组聚合为单个 PR（`patterns: ["*"]`）。
- `security-updates`：
  - 同一生态的安全更新按组聚合为单个 PR（`patterns: ["*"]`）。

## 目的

- 避免一次检查生成大量分散的依赖更新 PR。
- 保留依赖更新自动化，同时把评审入口收敛到“按生态聚合”的少量 PR。

## 说明

- 修改配置后，只影响后续 Dependabot 运行。
- 当前已经打开的 Dependabot PR 不会因为配置变更自动关闭，仍需手动合并或关闭。
- CI 的 Linux 原生依赖属于 runner 环境约束，新增 Rust/Tauri Linux 依赖时应先同步评估是否需要扩充该安装清单。
