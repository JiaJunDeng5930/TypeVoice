# TypeVoice 任务拆解 v0.1（Windows MVP）

说明：

- 本任务列表以 `docs/roadmap-v0.1.md` 为里程碑蓝图。
- 每个任务都要能支撑 `docs/verification-v0.1.md` 的 `quick/full` Gate。
- 冻结决策：ASR Runner 形态选择 A（依赖本机 Python/conda 环境，通过 `python ...` 启动）。

---

- [ ] 1. M1：ASR Runner 垂直切片（无 UI）
- [ ] 1.1 定义 ASR Runner 的输入输出协议（JSON）
  - 定义请求结构：`task_id`、`audio_path`、`language=zh`、`device=cuda`、`dtype=fp16`、`decode_params`
  - 定义响应结构：`ok`、`text`、`segments?`、`metrics{rtf,elapsed_ms,audio_seconds,device_used,model_id,model_version}`、`error{code,message,details?}`
  - 明确“不允许 CPU 降级”：若 cuda 不可用必须返回失败（明确错误码）
  - _Requirements: `docs/verification-v0.1.md`#3, `docs/tech-spec-v0.1.md`#4_
  - Completion: 协议写入文档（可先写在 `docs/tech-spec-v0.1.md` 的 ASR 接口章节里或单独小节）
- [ ] 1.2 选择并冻结 ASR Runner 的 Python 依赖集（最小可运行）
  - 明确 Python 版本范围（例如 3.10/3.11）与安装方式（conda/venv 均可）
  - 明确 PyTorch CUDA 安装口径（只要求“本机可用”，不做发布级打包）
  - 明确模型运行所需库（transformers/accelerate/torchaudio 等，具体以后以实测为准）
  - _Requirements: `docs/base-spec-v0.1.md`#3.2, `docs/tech-spec-v0.1.md`#4.1_
  - Completion: 在文档中列出“最小依赖清单 + 失败时诊断步骤（cuda 不可用如何判断）”
- [ ] 1.3 实现 ASR Runner（独立进程）
  - 入口建议：`asr_runner/runner.py`
  - 通信建议：stdin 读一条 JSON -> stdout 输出一条 JSON（单请求单响应，便于管理）
  - 强制：启动时检测 `torch.cuda.is_available()`；不可用直接错误返回
  - 记录：输出 metrics（rtf/device_used/elapsed_ms 等）
  - _Requirements: `docs/verification-v0.1.md`#4,#5_
  - Completion: 用 `fixtures/zh_10s.ogg` 能跑出非空 `text` 且 `device_used=cuda`
- [ ] 1.4 实现验证入口（CLI 或脚本）与指标落盘（JSONL）
  - 建议位置：`scripts/verify_quick.py`、`scripts/verify_full.py`
  - 输出：控制台摘要 + 追加 JSONL（例如 `metrics/verify.jsonl`）
  - 强制：若 `device_used!=cuda` 直接 FAIL
  - _Requirements: `docs/verification-v0.1.md`#2,#3_
  - Completion: 本机一条命令能跑完 quick/full 并给出 PASS/FAIL
- [ ] 1.5 取消机制（最小可验收版本）
  - quick/full 里对“取消”的实现可以先通过“父进程 kill runner”完成
  - 要求：取消延迟 <=300ms（以父进程触发 kill 到确认进程退出为准）
  - _Requirements: `docs/base-spec-v0.1.md`#5.1, `docs/verification-v0.1.md`#4,#5_
  - Completion: quick 中能在转录阶段触发取消并 PASS
- [ ] 1.6 fixtures 获取与本机约定固化
  - 已冻结：fixtures 放本机 `fixtures/` 且不提交 git
  - 提供可选下载脚本（便于 Windows 新机器一键拉取）：例如 `scripts/download_fixtures.(ps1|py)`
  - _Requirements: `docs/fixtures-sources-v0.1.md`, `docs/verification-v0.1.md`#2_
  - Completion: 新机器按文档能复原 fixtures（文件名一致）

- [ ] 2. M2：FFmpeg 预处理切片（与 ASR 串联）
- [ ] 2.1 冻结 FFmpeg 预处理参数与输出格式
  - 输出格式建议：WAV/PCM/16k/mono（最终以 ASR Runner 要求为准）
  - _Requirements: `docs/tech-spec-v0.1.md`#3, `docs/perf-spike-plan-v0.1.md`#5_
  - Completion: 参数写入文档并被验证脚本使用
- [ ] 2.2 将 full 验证改为“预处理 -> ASR”全链路测 RTF
  - 分离计时：preprocess_ms 与 asr_ms
  - _Requirements: `docs/perf-spike-plan-v0.1.md`#1,#2_
  - Completion: full 输出每条样本的两段耗时与 RTF，且达标
- [ ] 2.3 取消覆盖：预处理阶段取消
  - _Requirements: `docs/verification-v0.1.md`#5_
  - Completion: 预处理中取消 <=300ms 且 ffmpeg 进程停止

- [ ] 3. M3：最小桌面壳（Tauri + UI）
- [ ] 3.1 建立最小 UI：fixtures 转录按钮 + 阶段状态 + 文本展示 + 复制
  - _Requirements: `docs/base-spec-v0.1.md`#3.2_
  - Completion: 手工 Gate（<=2 分钟）通过
- [ ] 3.2 事件与指标在 UI 可见
  - 最近一次任务：阶段流转、耗时、错误码与摘要
  - _Requirements: `docs/base-spec-v0.1.md`#4.4_
  - Completion: 能在 UI 上定位失败原因（例如 cuda 不可用）

（M4-M6 任务拆解将在 M3 结束后补充，以避免过早设计录音与 LLM 模板的 UI 细节。）

