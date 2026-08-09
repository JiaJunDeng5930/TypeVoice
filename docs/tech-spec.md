# TypeVoice 技术规格

状态：已实现的技术契约；跨平台 adapter 仍必须由 Windows/Linux/macOS gate 分别证明。

范围：Windows 与 macOS 桌面端的线协议、能力边界与工程约束，并保留冻结的 Linux 自动输入 adapter 合同。业务状态、转移和取消语义只在 `docs/architecture.md` 定义；本文件不复制第二份状态机。

## 1. 总体架构

系统由前端或后端热键适配器把 Primary/Cancel intent 直接交给 `WorkflowController`，Controller 在同一串行 turn 内解释动作并提交业务状态，单次运行的 `RunExecutor` 执行录音、转录、改写、插入和持久化。前端只渲染 Controller 快照，异步结果不能经前端回报后端。

```text
Frontend -- Primary(actionKey) / Cancel(targetRunId) --> Tauri commands
                                                |
backend hotkey adapter -------- Primary --------+--> WorkflowController
                                                       |
                                                       v
                                               RunExecutor(runId)
                                                       |
                                                       v
                                  recording / ASR / rewrite / insertion / history ports

WorkflowController
-> workflow.state snapshot
-> UiEventMailbox
-> every frontend window
```

模块边界如下：

- `apps/desktop/src` 发送 intent、按 revision 渲染快照，并独立读取/复制 History 或手动导出 lastRun.recoveredResult。
- `apps/desktop/src-tauri` 只做 Tauri 参数映射、依赖注入和热键适配。
- `crates/typevoice-engine` 持有 `WorkflowController`、纯转移规则、`RunExecutor` 和本次运行计划。
- `crates/typevoice-core` 定义结果、run signal 和能力端口。
- `crates/typevoice-platform`、`crates/typevoice-providers`、`crates/typevoice-storage`、`crates/typevoice-observability` 分别实现平台资源、provider、存储和观测适配器。

Controller 是唯一业务状态 owner，但不执行网络、文件、FFmpeg、History 或平台输入 I/O。Executor 是一个 `runId` 的资源 owner，持有统一 cancellation token、可等待的 resource handles、resource/stage/cancel/terminal/finalization arbiter 和 completion guard；只要进程继续服务，它为每个已接受 Primary/Start 恰好产生一个资源已释放的 typed `Stopped`，无法证明释放时则进入明确 fatal shutdown 且不伪造 terminal。

## 2. 命令和快照协议

工作流业务面只保留两个 Tauri 命令：

- `workflow_snapshot() -> WorkflowView`
- `workflow_command(Primary { actionKey } | Cancel { targetRunId? }) -> WorkflowCommandReply`

`WorkflowCommandReply` 包含完整 `view` 和 `disposition: Applied | NoOp | CancelTooLate`；未知或畸形 intent 返回结构化 error。Primary 只由 Controller 在当前串行 turn 内按 Ready/Recording/Processing/Cancelling 解释为 Start/Stop/Cancel/NoOp，UI 和 hotkey adapter 不读取 phase 后改发另一个命令。窗口 Primary 原样回传 `WorkflowView.actionKey`；该 key 从 expected action 与当前/前序 run 身份纯派生，普通 progress 不改变它，mode 或 run 身份变化会自然改变它。显式 Cancel 携带 targetRunId，同一 run 跨 Recording/Processing/Finalize 仍进入 arbiter，旧 run 请求才 NoOp。Backend hotkey 使用当前动作，但 adapter 必须把一次完整按下/释放去重成一个 Primary，过滤自动重复和重复 callback。显式 Cancel 保留给独立取消手势，用于区分 Recording Stop 与放弃 run，不增加第二个主按钮。

`WorkflowView` 至少包含：

- 单调 `revision`；
- 从状态纯派生的 `actionKey = Start(Initial|After(lastRun.runId)) | Stop(runId) | Cancel(runId) | NoOp(runId)`；
- `mode`、当前 `runId?` 和内部 `stage?`；
- `lastRun?`，其中包含 outcome、可保留结果、warning、primary error、recovery errors 和可选 protocolContext；
- `primaryLabel`、`primaryDisabled` 和 `cancelEnabled`。

未知 mode、未知 intent 或缺少必填字段必须返回结构化错误并 fail closed。与当前派生值不同的 actionKey 或 mismatched targetRunId 按 `docs/architecture.md` A1 返回 NoOp；前者不能跨 mode 把 Stop 变成 Cancel，也不能让旧 Ready 窗口在后续 Ready 意外启动新 run，后者不能让旧 run 的取消命中新 run。

`rewriteLast`、`insertLast` 和 `copyLast` 不属于目标工作流命令。当前 UI 没有 rewrite/insert 的独立决策点，冻结产品流程又要求 stop 后自动完成 ASR、可选改写、剪贴板复制和可选自动粘贴，所以这些能力由单次 executor 编排。History 的手动复制是已提交记录的独立读/导出操作；若恢复性 History 写入也失败，Record 页直接从 `lastRun.recoveredResult` 显示“未保存”文本并调用同一剪贴板导出能力，这个动作不进入 executor、也不改变 workflow。`workflow_apply_event`、`workflow_report_*` 以及 `record_transcribe_*`、`rewrite_text`、`insert_text` 兼容入口在迁移后删除，不能保留成第二条终态或能力通道。

## 3. 单次运行协议

Controller 在 Ready 接受 Primary 时生成唯一 `runId`，并从已缓存、已验证的 settings 复制纯 `RunPlanSeed`。Seed 穷尽包含会改变本次 run 的非敏感配置：ASR provider/remote URL/model/concurrency、预处理参数、录音设备选择策略、LLM base URL/model/reasoning/prompt、rewrite/glossary/vision、auto-paste、context policy 和 intent source；各 capability 收到从 seed 派生的不可变配置。Controller 不读取 settings 文件、窗口、剪贴板、截图、API key 或上下文；secret 只保留安全存储引用，由对应 capability 按需读取。运行中修改非敏感设置只影响下一次 Start。

每个 active run 恰有一个 executor。Start 通过只分配内存控制面、不返回业务错误的构造器同步创建 dormant handle；跨边界全序固定为 state/revision commit → Begin attempt 与有界 BeginAccepted/失败 → nonblocking `workflow.state` enqueue → command reply，actionKey 只从提交后的状态投影。BeginAccepted 表示 executor 已在 R1 后捕获目标窗口身份、实际开始音频采集并注册录音 handle，还在 Doubao plan 下启动并注册受同一 token/arbiter 管理的 WebSocket resource handle；Primary 到实际采集必须 <=200ms。网络连接 future 可以在已注册 handle 内继续，不能阻塞该延迟；完整上下文读取也继续作为可取消 stage。Begin 在事件 sink 之前，所以窗口断开不能阻止 executor；Begin/Stop 投递、录音/streaming/ContextCapture 启动失败都走统一 typed Failed terminal。

Executor 终止信号是 outcome-discriminated union：`Stopped(runId, terminal)`，其中 `Completed { result, warning? }` 必带 ASR/final/timing/InsertResult，`Empty { timings }` 不允许 result/error，`Failed { error, recoveredResult?, recoveryErrors[] }` 必带 primary error 并可追加恢复错误，`Cancelled { recoveredResult?, cleanupDiagnostic? }` 只带允许恢复的数据。非法字段组合不能构造或反序列化；Cancel Accepted 后 executor 只能构造 Cancelled。

顶层 wrapper 的 completion guard 保证正常 terminal 与 supervisor 合成的 terminal 只有一个胜出。Empty、Finalize 前 Failed 和 supervisor Failed 必须先经同一 arbiter 的 `begin_terminal`；不可逆提交先经 `begin_finalization`，两者都与 Cancel 线性化。Inner panic、control channel close 或未提交结果便退出时，supervisor 先读取既有 winner：尚无 winner 才领取 `begin_terminal(Failed)`，ordinary/finalization winner 沿已有所有权收敛为 Failed，Cancel winner 则只追加 cleanupDiagnostic 并在有界 cleanup 后发 Cancelled。进程继续服务时，每个已接受 Primary/Start 恰好一个 Stopped。Controller 忽略不匹配或重复的 signal，且不得因此更新 revision、写 History、复制文本或广播终态。

内部 `Progress(runId, payload)` 也是 typed union：ContextCapture、RecordFinalize、Preprocess、Transcribe、Rewrite、InsertPrepare 各有 Started/Completed 及对应 result，Finalize 只有 gate 后的 Started。Recording 接受 ContextCapture；Primary Stop 时 Controller 按 context 子状态把 Processing 首 stage 设为 ContextCapture Pending/Running 或 RecordFinalize Pending，前者 Completed 后才前移，因此公开 stage 不回退。之后严格按 `RecordFinalize -> Preprocess -> Transcribe -> Rewrite? -> InsertPrepare -> Finalize` 推进；Rewrite 失败且 finalization 胜出时可直接进入 Finalize，以保存 ASR 后 Failed。重复、回退、其他跳 stage 或 payload/state 不匹配以 `E_EXECUTOR_PROGRESS_ORDER` fail closed 且不增 revision；partial text 和 audio level 是独立显示事件，不进入 reducer。

## 4. 录音和转录规范

录音与转录属于同一 run。Executor 负责：

- 在 R1 后捕获目标窗口、截图/剪贴板等允许上下文，失败走 typed Failed；
- 创建并持有录音/FFmpeg handle，以及 Doubao plan 的 streaming session；
- 在 Stop 后完成录音收尾和 FFmpeg 预处理；
- 选择 ASR provider，并把统一 cancellation token 传入本地、HTTP 或 streaming session；
- 把 provider 结果规范化为统一 `TranscriptionResult`；
- 记录 stage timing，并把 ASR result 留在该 run 的内存结果中；
- 在所有终态清理音频中间产物。

Doubao provider 使用 WebSocket 流式转录，凭据通过 keyring 或环境变量读取。远程 HTTP provider 沿用现有配置和 API Key 管理。Provider 只返回能力结果，不推进 workflow、不写 History，也不直接向 UI 发送业务终态。

Doubao session 在 Begin 注册并于 Recording 期间只把 partial 作为 display event；Stop 后的 Transcribe 只 finalize/await 既有 session，不二次 launch。远程 HTTP request 才在 `begin_stage(Transcribe)` 启动。两者都服从同一 token；窗口仅显示当前 runId 的 partial。空 ASR 只允许从 Transcribe 以 typed Empty 结束，不进入 rewrite/InsertPrepare/Finalize，也不创建空文本 History。预处理或 ASR 失败用 typed Failed 结束，已经得到的数据放入 recoveredResult。

## 5. 改写规范

Rewrite 是 `RunPlanSeed` 控制的可选内部 stage，不是前端单独调用的 workflow 命令。Executor 把 ASR 文本、R1 后捕获的上下文和缓存提示词一次性传给 LLM port，并把同一 cancellation token 传入请求。

Rewrite capability 只返回改写结果，不写 History、不改业务状态。改写失败时，若 Cancel 没有先胜出，Executor 获得 finalization gate、报告 Finalize::Started，再把 ASR 原文写入该 run 唯一的 History record，以 Failed 结束且不执行 run 内自动复制；如果恢复性 History 也失败，Rewrite 保持 primary error，History error 进入 recoveryErrors，ASR 留在 recoveredResult，Record 失败投影必须标明“未保存”并提供对该结果的手动复制。Cancel 先胜出时不写 History，只以 Cancelled 结束。

## 6. 插入规范

非空成功 run 必须执行一次 InsertPrepare 和一次剪贴板复制；可选的只有 `auto_paste_enabled` 控制的平台自动输入。Executor 只有在 terminal gate 已线性化且唯一 History commit 成功后才复制。Windows 使用平台 Unicode input capability，Linux 使用 AT-SPI adapter，macOS 使用 Core Graphics Unicode 键盘事件；三者都禁止用 `Ctrl+V` 或 `Cmd+V` 等快捷键模拟，并服从相同的 `InsertionPort` contract tests。

结果继续区分复制与自动写入：copy 失败产生 Failed；copy 成功但自动写入失败时，CompletedRunResult 带 `copied=true`、`autoPasteAttempted=true`、`autoPasteOk=false` 和 warning。平台 capability 不写 History、不改业务状态，且 copy/auto-paste 对一个 run 各最多一次；InsertResult 保存在 lastRun 与 trace，不引入第二次 History 更新。

## 7. 取消和副作用规范

Recording 与 Finalize 前的所有 Processing stage 都接受 Cancel。Outer handle 的本地全函数 arbiter 串行化 Begin resource registration、`request_cancel`、每个可取消 `begin_stage`、`begin_terminal` 和 `begin_finalization`，只返回 `Accepted | TooLate`。Launch 已先发生时 Cancel 仍 Accepted 并用 token/drop 终止它；Cancel 先到时后续工作或 terminal claim 不得启动。只有 terminal/finalization claim 已先胜出才 TooLate；Accepted reply 先于 completion resolve，Controller 在读取下一 signal 前提交 Cancelling。Accepted 后只允许 Cancelled，其他 variant 按 `E_EXECUTOR_TERMINAL_AFTER_CANCEL` 终止为协议 Failed；Recording/Processing 直接收到 Cancelled 则按 `E_EXECUTOR_CANCEL_UNACKNOWLEDGED` 失败。

Executor 获得 finalization gate 后立即报告 Finalize::Started，Controller 使 `cancelEnabled=false`。这不是第五个业务状态，而是 terminal ownership 已固定后的有界提交/清理子阶段；具体 Completed/Failed 仍由 History/copy/auto-paste 结果决定。与 `base-spec` 对齐的边界是：ContextCapture、Recording、ASR、Rewrite、InsertPrepare 等可逆用户工作 stage 均可 Cancel；History/copy/auto-paste 的不可逆 gate 已线性化后只能 TooLate，因为当前平台无法可靠回滚跨存储和操作系统输入的副作用，不能用补偿或假 Cancelled 隐藏。

History、provider 或平台 I/O 不得在 Controller 状态锁或 reducer 内运行。每个 capability/effect call 都有 operation deadline；用户主动保持的 Recording/streaming 用 heartbeat/idle 和 Stop/Cancel 后终止 deadline，而不是固定总时长。Finalize 的 History/copy 不得无限阻塞 TooLate；超时进入 terminal/cleanup。每个 resource port 还必须提供有 deadline 的 graceful shutdown 与 force abort/kill+wait；强制释放成功后，Cancel winner 用 Cancelled.cleanupDiagnostic，其他 winner 用 Failed。若 force 后仍无法证明资源释放，必须记录 fatal、终止进程并要求重启，不能回 Ready、接受新 run 或伪造 Stopped。取消总清理仍必须满足 300ms。

## 8. 事件规范

`workflow.state` 是唯一业务状态事件，payload 是完整 `WorkflowView`。普通转移先提交 revision，再从新状态派生 actionKey 并非阻塞广播；R1 的全序进一步固定为 commit → Begin attempt/有界 ack 或失败 → event enqueue → command reply，sink 失败只记显示诊断，不能阻止 executor。窗口挂载或重连时必须先注册 listener，再调用 `workflow_snapshot`。

`audio.level`、`transcription.partial` 和可选的 `workflow.progress` 是显示事件，只携带 `runId`、typed stage progress 和时间戳；窗口只显示与当前 WorkflowView.runId 相同的 transient event。它们不能带 `stateChanging` 语义，不能由前端回流 Controller，也不能触发 rewrite、copy、History 或终态。每个 listener 生命周期把 `latestRevision` 初始化为 None；第一份合法 view 包括 revision 0 都接受，之后只在 `revision > latest` 时应用并忽略 `<=`。若新事件先于 snapshot 到达，旧 snapshot 被自然忽略；命令 reply 的 disposition 始终处理，但相等 revision 的 view 不重复替换。

目标协议不再使用 `effect`、用于业务去重的 UI `eventId`，或 `stateChanging -> workflow_apply_event` 往返。终态 toast、History refresh 和 Overlay final text 都从新 revision 的 `lastRun` 派生，避免多个窗口分别解释完成事件。

## 9. 存储规范

History 以 `runId` 作为业务关联键；现有物理字段 `task_id` 在迁移期承载同一语义。中间 ASR/rewrite 结果只保存在当前 run，Executor 获得 terminal finalization gate 后才通过幂等 History port 最多提交一条完整 record；Empty 或 Cancel Accepted 不写，任何 capability 都不能直接写入。

历史记录至少保留 `created_at_ms`、`asr_text`、`final_text`、兼容 `template_id`、`preprocess_ms`、`asr_ms`、`rtf` 和 `device_used`。Rewrite 失败提交包含 ASR 的唯一 record；若恢复性 History 也失败，primary Rewrite error 与 recovery History error 同时进入 lastRun，ASR 保留在 recoveredResult，且不开始 run 内 automatic copy。Record 页仍允许用户用独立剪贴板导出动作复制该 recoveredResult，并明确显示它没有持久化。音频中间产物必须在任何非 fatal terminal 前清理。

## 10. 配置、错误和观测

普通配置写入 `settings.json`，API Key 使用系统安全存储或环境变量，敏感字段不得写入日志或事件。

所有可见失败都包含 primary error code/摘要及有序 recovery errors；Controller 因非法 terminal 生成协议 Failed 时，另用 `protocolContext { receivedTerminal, receivedError? }` 保存原 variant/error，不能混入 recoveryErrors。每次真实状态提交记录 `workflow.transition`，字段至少为 `runId, revision, from, to, cause, stage?, outcome?, errorCode?, recoveryErrorCodes?, protocolTerminal?, protocolErrorCode?`；executor 记录 `run.terminal_won`/`run.finalization_won`、每次 `run.cancel_arbitrated`，以及 cleanup 的 escalated/forced/fatal 结果。Stage、资源、History 和 provider trace 使用同一 `runId`；资源 ID 可作为附加 context，但不能形成第二条业务因果链。

实现验收必须执行 `docs/architecture.md` 第 8 节的场景和不变量，包括 Primary 到实际采集 <=200ms、capture-after-commit、actionKey/targetRunId admission、cancel-vs-terminal、typed signal、失败结果手动复制、bootstrap revision、cleanup fatal containment 和 Windows/Linux/macOS InsertionPort。测试选择器必须证明匹配并执行了非零 workspace/engine contract tests；0-test 或只在单一 OS 运行其他平台 Skipped 都不能视为完整 gate 通过。
