# 发布流程

本项目采用 SemVer 与 Keep a Changelog。

## 1. 当前状态

- 当前处于 `0.x` 阶段。
- 尚未发布首个正式版本 tag。
- 所有未发布变更记录在 `CHANGELOG.md` 的 `Unreleased` 区块。

## 2. 发布前检查

- 更新 `CHANGELOG.md`。
- 通过仓库验证命令：
  - `./.venv/bin/python -m pytest -q tests`
  - `./.venv/bin/python scripts/verify_quick.py`
  - `./.venv/bin/python scripts/verify_full.py`（环境允许时）
- 确认许可证与第三方声明文件仍然完整：
  - `LICENSE`
  - `THIRD_PARTY_NOTICES.md`

## 3. 首次发布建议步骤

1. 将 `Unreleased` 内容整理为 `0.1.0` 条目。
2. 打 tag：`v0.1.0`。
3. 在 GitHub Releases 发布对应 release notes。

## 4. 自动化

- `.github/release.yml` 用于 release notes 分类。
- `.github/workflows/ci.yml` 提供基础 CI 信号。
