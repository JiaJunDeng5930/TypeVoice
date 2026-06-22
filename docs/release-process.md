# 发布流程

本项目采用 SemVer 与 Keep a Changelog。

## 1. 当前状态

- 当前处于 `0.x` 阶段。
- 尚未发布首个正式版本 tag。
- 所有未发布变更记录在 `CHANGELOG.md` 的 `Unreleased` 区块。

## 2. 发布前检查

- 更新 `CHANGELOG.md`。
- 通过仓库验证命令：
  - `cargo test --locked -p xtask`
  - `cargo xtask verify quick`
  - `cargo xtask verify full`（环境允许时）
- 确认许可证与第三方声明文件仍然完整：
  - `LICENSE`
  - `THIRD_PARTY_NOTICES.md`

## 3. 首次发布建议步骤

1. 将 `Unreleased` 内容整理为 `0.1.0` 条目。
2. 打 tag：`v0.1.0`。
3. 推送 tag：`git push origin v0.1.0`。
4. 等待 CD workflow 创建 GitHub Release，并上传 Windows `.exe` / `.msi` 安装包。

## 4. 自动化

- `.github/release.yml` 用于 release notes 分类。
- `.github/workflows/ci.yml` 提供基础 CI 信号。
- `.github/workflows/cd.yml` 在 `v*` tag 推送时打包 Windows 安装包并发布到 GitHub Releases。
