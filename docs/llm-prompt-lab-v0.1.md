# LLM Prompt Lab v0.1

目标：把“测试 LLM 返回 -> 调整 prompt -> 再测”的流程固化为可复用的脚本，不做自动判定（人工判断输出是否符合预期）。

脚本：`scripts/llm_prompt_lab.py`

## 1. 基本用法

最小示例（只发 transcript）：

```bash
python3 scripts/llm_prompt_lab.py \
  --base-url http://api.server/v1/chat/completions \
  --model gpt-5.3-codex \
  --reasoning-effort medium \
  --system-prompt-file tmp/prompt_under_test.txt \
  --transcript "现在进行第一条测试，用于验证效果。"
```

说明：

- `--base-url` 可填 `.../v1` 或完整 `.../v1/chat/completions`，脚本会自动归一化并发送到 `/chat/completions`。
- API Key 默认从环境变量 `TYPEVOICE_LLM_API_KEY` 读取（不建议用 `--api-key`，避免落到 shell history）。
- 每次运行会落盘到 `tmp/llm_prompt_lab/<timestamp>_<hash>/`：
  - `meta.json`、`request.json`、`response.json`、`response.txt` 等。

## 2. 带上下文的注入方式（对比实验）

当前 app 行为（一个 user message 内同时含 transcript + context）：

```bash
python3 scripts/llm_prompt_lab.py \
  --base-url http://api.server/v1 \
  --model gpt-5.3-codex \
  --reasoning-effort medium \
  --system-prompt-file tmp/prompt_under_test.txt \
  --inject-mode inline_one_user \
  --transcript "现在进行第一条测试，用于验证效果。" \
  --history-file tmp/history.txt \
  --clipboard-file tmp/clipboard.txt \
  --prev-process "C:\\Windows\\explorer.exe"
```

对比模式（拆成两条 user message：第一条只给 transcript，第二条只给 context，并带“不要复述上下文”的固定前缀）：

```bash
python3 scripts/llm_prompt_lab.py \
  --base-url http://api.server/v1 \
  --model gpt-5.3-codex \
  --reasoning-effort medium \
  --system-prompt-file tmp/prompt_under_test.txt \
  --inject-mode two_user_messages \
  --transcript "现在进行第一条测试，用于验证效果。" \
  --history-file tmp/history.txt \
  --clipboard-file tmp/clipboard.txt \
  --prev-process "C:\\Windows\\explorer.exe"
```

`tmp/history.txt` 约定：每行一条历史记录文本（脚本会做截断与拼接）。

## 3. 快速迭代 prompt（可选）

如果你希望每次运行前都打开编辑器修改 prompt：

```bash
export EDITOR=vim
python3 scripts/llm_prompt_lab.py \
  --base-url http://api.server/v1 \
  --model gpt-5.3-codex \
  --reasoning-effort medium \
  --system-prompt-file tmp/prompt_under_test.txt \
  --edit \
  --transcript "现在进行第一条测试，用于验证效果。"
```

## 4. 注意事项

- 本脚本不做“输出是否正确”的自动判定：你需要人工判断 `response.txt` 是否符合预期。
- 脚本不会将 API Key 写入磁盘（但若你用 `--api-key`，它仍可能出现在终端历史中）。

