# 仓库自动化

## Dependabot

- 配置文件：`.github/dependabot.yml`
- 监控范围：
  - `npm`：`/apps/desktop`
  - `cargo`：`/apps/desktop/src-tauri`
  - `pip`：仓库根目录

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
