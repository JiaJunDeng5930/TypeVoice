# TypeVoice

面向中文输入场景的桌面端“录完再出稿”语音输入工具。

## 功能概览

- 本地录音 -> FFmpeg 预处理 -> ASR 转写 -> 可选 LLM 改写 -> 复制/自动粘贴
- 历史记录仅保存文本与元信息，不保存音频
- 支持 Doubao 流式 ASR 与远程 HTTP ASR
- 提供 `quick/full` Rust 验证工具

## 快速开始

### 1) 安装前端依赖

```bash
cd apps/desktop
npm ci
```

### 2) 下载验证 fixtures（建议）

```bash
cd /path/to/TypeVoice
cargo run --locked -p typevoice-tools -- fixtures download
```

### 3) 启动桌面应用

```bash
cd apps/desktop
npm run tauri dev
```

## 常用验证命令

在仓库根目录执行：

```bash
cargo test --locked -p typevoice-tools
cargo run --locked -p typevoice-tools -- verify quick
cargo run --locked -p typevoice-tools -- verify full
```

说明：`verify quick` / `verify full` 会按 `scripts/fixtures_manifest.json`
自动下载并校验缺失的 fixture 音频。

Windows 一键门禁参见 [docs/windows-gate.md](./docs/windows-gate.md)。

## 使用说明

1. 开始录音
2. 结束录音
3. 等待转写（可选改写）
4. 复制结果或自动粘贴到目标输入框

## 隐私与数据流

- 使用 Doubao 流式 ASR：录音音频会发送到 Doubao ASR 服务。
- 使用远程 HTTP ASR：音频会发送到你配置的远程 ASR 服务。
- 启用 LLM 改写：仅发送转写文本与必要上下文，不发送音频。
- API Key 存储在系统安全存储（keyring）中，不写入日志。

详细说明见 [docs/privacy-data-flow.md](./docs/privacy-data-flow.md)。

## 反馈与支持

- Bug/需求：<https://github.com/JiaJunDeng5930/TypeVoice/issues>
- 支持边界：见 [SUPPORT.md](./SUPPORT.md)
- 安全问题：见 [SECURITY.md](./SECURITY.md)

## 贡献

贡献流程见 [CONTRIBUTING.md](./CONTRIBUTING.md)。

## 文档导航

- 文档索引： [docs/index.md](./docs/index.md)
- 基础规格： [docs/base-spec.md](./docs/base-spec.md)
- 技术规格： [docs/tech-spec.md](./docs/tech-spec.md)
- 验证规范： [docs/verification.md](./docs/verification.md)
- 发布流程： [docs/release-process.md](./docs/release-process.md)

## 许可证与第三方声明

- 本项目许可证： [LICENSE](./LICENSE)（MIT）
- 第三方组件声明： [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)
