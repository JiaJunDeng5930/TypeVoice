# TypeVoice 目标架构与工作流契约

状态：已实现的工作流契约。当前业务代码和自动化验收以本文为唯一工作流状态来源；`docs/base-spec.md` 仍是产品行为上位约束，`docs/tech-spec.md` 只保留线协议和适配器细节。T23 的 Windows/Linux adapter 必须继续由各自平台 gate 分别证明，未执行的平台不能记为 PASS。

本文解决一个具体问题：重构前，一次语音输入同时由后端 phase、多个全局资源槽、两个前端窗口的本地状态和异步回报命令共同推进，所以维护者无法从一个位置回答“当前业务状态是什么、这个结果还应不应该生效、用户看到的错误属于哪次运行”。当前设计把状态判断收回一个后端所有者，同时把 FFmpeg、ASR、LLM 和插入资源留在单次运行边界内，不把执行细节集中到长期存活的 god actor。

## 1. 重构前实现事实基线

本节记录 `refactor/architecture-state-machine` 分支、HEAD `3f6bc24` 在 2026-07-13 的直接观察。行号用于复核该基线，不是目标 API。

### 1.1 已直接观察到的事实

| 事实 | 当前证据 | 对业务链路的直接影响 |
| --- | --- | --- |
| 实时 Git 树仍是 workspace crate 结构 | `git ls-tree -r --name-only HEAD` 将核心文件列在 `crates/typevoice-engine/src/`；`apps/desktop/src-tauri/src/lib.rs:3-5` 只是 re-export 组合层 | 该基线的 AGENTS 自动 project index 所列路径已经陈旧，迁移完成后必须由生成器刷新 |
| 后端已有一个名义上的状态真相 | `voice_workflow.rs:23-33` 定义九个 phase，`232-287` 用 `Mutex<WorkflowState>` 持有 session、结果、错误和事件缓存 | phase 的写入集中在 `VoiceWorkflow`，但异步结果能否到达它并不由它控制 |
| ASR 终态由两个前端窗口重复回报 | `MainScreen.tsx:140-223` 与 `OverlayApp.tsx:205-276` 都监听 `ui_event`，并调用 `workflow_report_asr_completed/empty/failed` | Main 或 Overlay 的生命周期、监听顺序和 Promise 处理会决定后端能否离开 `Transcribing`；两个窗口还会竞争提交同一结果 |
| 文档所写的 `stateChanging -> workflow_apply_event` 链不存在 | `ui_events.rs:78,99,119,137,166,183,209,235,252` 的所有事件都是 `displayOnly`；前端没有 `workflow_apply_event` 调用；`voice_workflow.rs:328-341` 只按 event ID 缓存并原样返回当前 view | `effect` 和 event ID 构成没有状态作用的第二套协议，不能提供文档声称的终态提交或晚到保护 |
| 不同执行路径采用不同终态所有者 | `voice_tasks.rs:37-139` 的 rewrite/insert task 直接调用 `workflow.report_*`；`145-234` 的 stop/transcribe task 只发 UI 事件；`voice_workflow.rs:873-1117` 又保留直接 await 并改状态的 rewrite/insert 路径 | 同一种“异步完成”需要维护三种提交方式，新增阶段时无法沿一个模板扩展 |
| 自动流程实际由 Main 前端编排 | `MainScreen.tsx:37-103` 在 `Transcribed/Rewritten` 后根据前端 settings 副本调用 `workflow_rewrite` 和 `workflow_insert` | `Transcribed`、`Rewritten` 成为等待前端继续调用的中转 phase；窗口缺席或前端异常会让业务流程停住 |
| 前端会覆盖后端动作投影 | `voice_workflow.rs:1205-1216` 计算按钮状态；`MainScreen.tsx:274-278` 又按 phase 改写 command、label 和 disabled | 后端 snapshot 不是完整 UI 真相，新增或重命名 phase 时必须同步修改前端条件分支 |
| 取消不满足冻结产品规格 | `base-spec.md:36,81,113,160` 要求任何阶段取消；`voice_workflow.rs:817-870` 只接受 Recording/Transcribing，并拒绝 Rewriting/Inserting | 当前不能把“可取消”作为一次运行的统一不变量，LLM 和插入阶段也没有共同 cancellation seam |
| 异步任务和运行资源分别由多个全局槽持有 | `voice_tasks.rs:16-17` 丢弃 spawn handle；`audio_capture.rs:104-121`、`transcription.rs:117-129`、`transcription_actor.rs:75-88` 各自持有一个 active/session 槽 | 状态 owner 没有一个可 join 的单次运行句柄；替换、取消和晚到判断分散在各模块 |
| 持久化提交点不唯一 | `rewrite.rs:98-104` 已更新 History，随后 `voice_workflow.rs:911-913,1290-1299` 再更新一次；ASR report 在 `742-749` 先改内存状态再写 History | 重复写入没有业务价值；History 失败还会形成“后端已转移、UI 未收到新 snapshot”的部分提交 |
| `insert_previous_phase` 没有读取者 | 字段定义和写入位于 `voice_workflow.rs:239,1759`，全仓搜索没有读取路径 | 该字段是没有真实职责的状态数据，不应迁移到目标模型 |
| 当前 trace 有相关字段，但因果链并不连续 | `obs/schema.rs:27-38` 已有 `task_id/stage/step_id`；`commands.rs:338-352` 的 workflow command 错误却记录 `task_id=None`，`audio_capture.rs:322-330` 的 stop span 也不带业务 ID | 用户动作、资源操作和终态无法稳定通过一个 ID 连接，排障仍需按时间猜测 |
| 现有 verify 可能 0-test 通过 | `xtask/main.rs:506-541,606-623` 在 `apps/desktop/src-tauri` 运行未带 `--workspace/-p` 的过滤测试；现场 `cargo test --locked --workspace` 列出 107 个通过测试，而 quick 的三个过滤器都显示 `running 0 tests` 后退出 0 | 该基线的 gate 通过不能证明 engine 状态或可调试性契约被执行，迁移必须把非零匹配数作为 gate 条件 |

全仓调用面还证明了命令契约的实际产品语义：Record 页只有单一录音按钮，Overlay 只有 `primary` 热键，History 复制直接读取持久记录；前端没有 `rewriteLast`、`insertLast`、`copyLast` 调用者，也没有 `workflow_report_rewrite_*`、`workflow_report_insert_*` 或 `workflow_apply_event` 调用者。当前 `workflow_rewrite/workflow_insert` 只是 Main 自动计划的两步。因此，`tech-spec` 旧文所称“改写、插入、复制由用户分别触发”被冻结的 `base-spec.md:24-29,50-57` 和实时 UI 同时推翻，不能作为保留无调用入口的理由。

### 1.2 基于事实的解释

这些事实共同说明问题需要架构重构，但重构面可以很小。九个 phase 并不是九种稳定业务选择，其中五个只是在描述执行到哪一步；ASR 前端回报和 Main 自动编排又把窗口变成事实上的第二 coordinator。只修某一个 report 命令会保留混合所有权，新增阶段时仍要复制同样的前后端协议。

三个全局 active 槽本身不等于错误，它们当前也有 stale ID 防护；问题在于这些槽与 detached task 没有共同的单次运行寿命。只有状态 owner 知道业务 run 是否有效，而只有各资源模块知道进程、token 或 session 是否已经释放，所以取消和替换无法由一个可检查的不变量描述。

### 1.3 设计基线中尚未证实且不作为前提的事项

- 本次没有通过人工操作复现 Overlay 文本重复、窗口关闭后卡在 Transcribing 或 cancel/completion 竞争；源码已证明这些路径存在，但发生频率仍未知。
- 该基线的 LLM 请求和平台插入没有 cancellation token。迁移必须用受控端口证明取消语义，设计不能假设原函数已可取消。
- 该基线中没有找到 AGENTS project index 的生成命令或配置，因此设计阶段没有手改自动块；迁移完成时已使用确认过的 project-index 生成器刷新索引。

## 2. 目标所有权与依赖方向

目标只增加两个清晰边界，不增加 manager、兼容通道或第二套状态。

```text
React UI -- Primary / Cancel + revision --> Tauri commands
                                                |
backend hotkey adapter -------- Primary --------+--> WorkflowController (single serialized owner)
                                                       | create/control one RunExecutor
                                                       v
                                                  RunExecutor(runId)
                         |
             recording / ASR / rewrite / insertion / history ports
                         |
                  typed run signals
                         v
WorkflowController -- atomic state + revision --> WorkflowView
                         |
                  UiEventMailbox
                         v
                 all frontend windows
```

**`WorkflowController` 是唯一业务状态所有者。** 它长期存活并串行处理 intent 与 run signal，只持有本文定义的状态、当前 `runId`、单次 executor handle、最近结果、最近 outcome 和单调 `revision`。它校验 `runId`，原子提交状态和 snapshot，然后才广播 `workflow.state`。它不在状态锁或 reducer 内执行 History、网络、文件、FFmpeg 或平台输入 I/O。

**`RunExecutor` 是一次录音的执行和资源边界。** Primary 在 Ready 被 Controller 解释为 Start 后，先通过不返回业务错误、只分配内存控制面的构造器同步创建 dormant handle；Controller 把它连同新 `runId` 原子提交为 Recording 后才发送 Begin。Executor 随后独占 cancellation token、录音/FFmpeg handle、ASR session 或 HTTP request、LLM request、临时音频和待完成的效果。它按该次 Start 冻结的纯 `RunPlanSeed` 执行，不直接改业务状态，也不向 UI 提交终态。Begin/Stop 控制投递失败与资源启动失败一样，由顶层 wrapper 收敛为 typed Failed terminal，不产生旁路错误。

Executor 顶层 wrapper 持有 completion guard：只要进程继续运行，每个已接受 Primary/Start 必须恰好产生一个 `Stopped`，并且只能在所有资源已 join/drop、临时音频已清理后产生。正常路径提交自身结果；inner task panic、控制通道关闭或未提交结果便退出时，wrapper 完成同样的资源清理并按 arbiter winner 合成 Failed 或 Cancelled terminal。它不是新的 manager，也不持有业务状态，只把一次 run 的所有退出路径收敛到同一 completion future。

`RunPlanSeed` 只同步复制 Controller 已缓存并验证、会改变本次 run 的非敏感配置：ASR provider/URL/model/concurrency，预处理参数，录音设备选择策略，LLM base URL/model/reasoning/prompt，rewrite/glossary/vision 开关，自动粘贴、上下文策略和 intent source。各 capability 只接收从 seed 派生的不可变配置；API key 不进入 seed，只保留安全存储引用并在对应 capability 启动时按需读取。R1 提交后，Executor 的 Begin 捕获目标窗口身份，启动并注册录音 handle，并在 Doubao plan 下启动、注册受同一 token 管理的 streaming resource handle；BeginAccepted 要求音频已经实际采集，且从用户 Primary 到此不得超过 200ms。Streaming handle 的连接 future 已受控即可，网络握手可以在该资源内继续并以统一 Failed 收敛，不能为了等网络而突破采集延迟；完整截图、剪贴板或文件读取也作为可取消 ContextCapture 继续。运行中修改这些非敏感设置只影响下一次 Start，所以同一 `runId` 的执行选择可以重放和解释，同时 Ready 不会在状态外创建平台资源。

`UiEventMailbox` 继续作为显示传输，不成为状态 owner。前端和 Overlay 只渲染 `WorkflowView` 及带当前 `runId` 的短暂音频/partial 数据；它们不能回报 ASR 结果、推进 rewrite/insert 或根据 phase 发明另一套按钮规则。窗口主按钮和全局热键只发送 `Primary`，由 Controller 在同一个串行 turn 内按当前状态解释为 Start、Stop 或 Cancel；独立取消手势发送显式 `Cancel`，用于区别 Recording Stop 与放弃本次 run，而不增加第二个主按钮。窗口 Primary 原样回传 snapshot 的 `actionKey`，显式 Cancel 携带所见 `runId`；后端热键在 Controller turn 内直接使用当前动作，因此没有“先读窗口状态、再发送另一个命令”的竞态。普通进度不改变派生 key，不会让仍然合法的 Primary 失效；同一 run 的显式 Cancel 即使与 Stop 或 Finalize 竞争，也会进入 executor arbiter，而不是被投影版本丢弃。

### 2.1 crate 职责

| 位置 | 目标职责 |
| --- | --- |
| `apps/desktop/src` | 发用户 intent，按 revision 渲染 snapshot；History 查询/复制与 recoveredResult 手动导出是独立读模型 |
| `apps/desktop/src-tauri` | Tauri 参数映射、依赖注入和 hotkey intent 适配，不含业务转移 |
| `crates/typevoice-engine` | `WorkflowController`、纯转移规则、`RunExecutor` 及执行计划 |
| `crates/typevoice-core` | run signal、结果和能力端口等无平台契约 |
| `crates/typevoice-platform` | 录音、FFmpeg、上下文、剪贴板和自动输入的 run-scoped handle |
| `crates/typevoice-providers` | ASR/LLM provider session；接受同一 cancellation token |
| `crates/typevoice-storage` | settings snapshot 与每个 run 的唯一 History 提交点 |
| `crates/typevoice-observability` | 使用 `runId` 的 transition、stage、error、metric 记录 |

## 3. 最小业务状态

目标状态只有四个：

```text
Ready { lastRun? }
Recording { activeRun }
Processing { activeRun, stage }
Cancelling { activeRun, stage }
```

Controller snapshot 只有一个存储计数器：`revision` 在每次实际投影提交时增加，供窗口拒绝乱序 snapshot。Primary admission 使用从当前状态纯派生、不单独存储的 `actionKey`：Ready 是 `Start(Initial | After(lastRun.runId))`，Recording 是 `Stop(activeRun.runId)`，Processing 是 `Cancel(activeRun.runId)`，Cancelling 是 `NoOp(activeRun.runId)`。Initial 只存在于进程尚未接受过 run 时；History 清空或 UI dismiss 不能清除 Controller 的 lastRun identity，runId 又永不复用，所以旧 Ready key 不会重新变成有效。普通 Context/stage progress 与 `Finalize::Started` 不改变 key；任何 Primary 语义、active run 或 Ready 前序 run 的变化都会自然产生不同 key，不存在需要手动同步的第二 revision。显式 Cancel 则携带 `targetRunId`，所以同一 run 从 Recording 进入 Processing 或 Finalize 后仍会得到 Accepted/TooLate 的真实仲裁结果。`activeRun` 至少包含 `runId` 和 executor handle；resource-local 的 recording ID、asset ID、provider request ID 只进入 executor 或 trace context，不能成为第二业务身份。

| 状态 | 保留它的必要性 | 对外动作 |
| --- | --- | --- |
| `Ready` | 没有活跃 executor，可以安全创建新 run。最近的完成、空结果、失败或取消作为 `lastRun.outcome/result/error` 展示，不改变下一步命令集合 | Primary 启动；Cancel 为幂等 no-op |
| `Recording` | 用户仍可说话，录音 handle 存活，Stop 有“结束采集并继续处理”的独有语义 | Primary 停止并继续处理；显式 Cancel 放弃本次 run |
| `Processing` | 录音已经结束，executor 正在完成预处理、ASR、可选改写和结果提交；不能创建第二条资源链 | Primary 或显式 Cancel 在 terminal gate 前请求取消；gate 已线性化时返回 TooLate |
| `Cancelling` | Cancel 已被接受但 executor 尚未确认资源释放。若直接并入 Ready，新 Start 可能与旧 FFmpeg、请求或插入效果并行 | 所有用户 intent 都 no-op，只等待匹配 run 的 `Stopped` |

### 3.1 内部 stage 不是业务状态

可取消的 executor stage 是 `ContextCapture`、`RecordFinalize`、`Preprocess`、`Transcribe`、可选 `Rewrite` 和 `InsertPrepare`。非空成功 run 必须经过 InsertPrepare，因为“关闭自动粘贴”只关闭平台自动输入，剪贴板复制仍是冻结产品行为；不存在“完全跳过 insertion”的 plan 开关。Doubao streaming session 是 Begin 注册的 Recording resource，它在 Recording 期间只产生带 runId 的 display partial；Stop 后的 Transcribe stage 只 finalize/await 并规范化该 session 的最终结果，不二次 launch。远程 HTTP ASR 则在 Stop 后通过同一个 `begin_stage(Transcribe)` 启动请求。

| 当前 mode | 合法 typed Progress 顺序 | 跳过与结果规则 |
| --- | --- | --- |
| Recording | 可选 `ContextCapture::Started -> Completed` | 窗口、截图、剪贴板和文件 capture 全部在 R1 后执行；不需要上下文时整段跳过 |
| Processing | 若 Stop 时 context 为 Pending/Running，公开 stage 从该精确子状态继续 `ContextCapture::Started? -> Completed`；否则直接从 `RecordFinalize` 开始，随后 `Preprocess -> Transcribe -> Rewrite? -> InsertPrepare -> Finalize::Started` | G1 根据 context 子状态选择初始 stage，绝不先公开 RecordFinalize 再回退；Rewrite 仅按 seed 跳过，或在失败且 finalization gate 胜出时直接转 Finalize 以保存 ASR 后 Failed；Empty 从 Transcribe 终止 |

Begin 对录音与 streaming session 的注册，以及每个可取消 stage 的 launch，都与 `request_cancel` 进入同一个 per-run arbiter。若 resource/stage launch 先线性化，后到 Cancel 仍可 Accepted，并通过 token/drop 终止已经启动的请求；若 Cancel 先线性化，后续 Context/ASR/LLM/InsertPrepare launch 被拒绝。Empty、Finalize 前 Failed 与 supervisor Failed 必须先调用 `begin_terminal`；`begin_finalization` 则在线性化不可逆 History/复制路径前调用。两种 terminal claim 只要先胜出，后到 Cancel 都 TooLate；Cancel 先胜出时 executor 只能产生 typed Cancelled。Finalization 胜出后立即发 `Finalize::Started` 并使 `cancelEnabled=false`，具体 Completed/Failed 仍由 History/copy/auto-paste 结果决定。

这与 `base-spec.md` 澄清后的可取消边界一致：所有可逆用户工作 stage 直到 InsertPrepare 都可取消；Finalize 是 terminal linearization 后的有界提交/清理子阶段，不是新的业务状态，也不能再诚实地声称取消成功。History 或平台输入已经开始后若仍返回 Cancelled，就必须增加跨存储与操作系统输入的补偿/回滚能力；当前平台没有这种能力，目标合同拒绝用假 Cancelled 掩盖已经发生的副作用。

Reducer 对 stage 协议严格校验：Recording 只接受 ContextCapture progress；G1 在 Stop commit 时按已观察的 context 子状态设置 Processing 的首个公开 stage，ContextCapture Completed 后才前移到 RecordFinalize Pending。其余 stage 只接受当前 plan 中严格下一项 Started/Completed；唯一旁支是 Rewrite 已 Started 后，executor 可在 rewrite 失败且 finalization gate 胜出时直接发一次 `Finalize::Started`，随后用 Failed terminal 表达原错误。重复、回退、其他跳 stage 或 payload/state 不匹配都归为 Invalid，记录 `E_EXECUTOR_PROGRESS_ORDER` 且不增 revision。Completed terminal 只允许在 Finalize，Empty terminal 只允许在 Transcribe；资源已经释放但 terminal 顺序错误时仍回 Ready(Failed/E_EXECUTOR_TERMINAL_ORDER)，避免因拒绝唯一 Stopped 而永久卡住。

### 3.2 合并证明

| 当前/产品概念 | 目标表示 | 合并理由及可观察后果 |
| --- | --- | --- |
| `Idle` | `Ready { lastRun: None 或上一结果 }` | 都表示没有活跃资源且可 Start；统一名称后不再用“是否刚完成”决定命令 |
| `Completed` | `Ready { lastRun.outcome = Completed }` | 完成只改变要展示和保存的数据，下一步仍是 Start；UI 从 outcome 显示成功，不需要一个阻止新录音的 phase |
| `Failed` | `Ready { lastRun.outcome = Failed, result?, error }` | 失败后仍只能开始新 run，ASR 原文和诊断作为结果数据保留；错误码不同不应扩张状态数 |
| `Cancelled` | `Ready { lastRun.outcome = Cancelled }` | 资源已释放后的取消与其他 Ready 拥有相同动作；取消过程本身由 `Cancelling` 表示 |
| `Transcribing`、`Rewriting`、`Inserting` | `Processing.stage` | Transcribe/Rewrite 与 InsertPrepare 共享同一 Cancel 和终止规则；当前 Inserting 中不可逆的 History/平台输入被收窄到 terminal Finalize，UI 仍从 stage 显示进度和耗时，但不会把 gate 后 TooLate 伪装成取消成功 |
| `Transcribed` | `Processing` 中已保存的 ASR result | 实时 UI 没有等待用户选择的控件，Main 只把它当作自动 rewrite/insert 触发器；后端 executor 接管 plan 后不再存在稳定停驻点 |
| `Rewritten` | `Processing` 中已更新的 final text | 实时 UI 只在此继续自动 insert，后续 intent 与 Transcribed 没有实质差异；文本来源保留在 result metadata |
| `Recording` | `Recording` | Stop 语义、活跃采集资源和用户反馈都与 Processing 不同，不能合并 |
| 取消请求到资源释放之间的瞬间 | `Cancelling` | 该阶段拒绝新 Start 且只接受清理确认，后继和外部行为与 Recording/Processing/Ready 都不同，因此是唯一需要新增的状态 |

History copy 不属于当前 run 状态机。它读取已经提交的记录，即使最近 run 是 Failed，也可以复制保留的 ASR 原文；若恢复性 History 写入本身失败，Record 页从 `lastRun.recoveredResult` 展示“未保存”的 ASR 并提供直接复制，这仍是现有结果的读/导出动作，不是 `copyLast` 工作流命令。没有真实 UI 调用者的 `rewriteLast/insertLast/copyLast` 不应为状态集合制造额外分支。

## 4. 完整事件与转移契约

### 4.1 输入字母表和判定顺序

Controller reducer 只处理五类输入，除此之外都属于 `Invalid`：

- 用户 intent：`Primary { actionKey }`、`Cancel { targetRunId? }`。Primary 在同一个 Controller turn 内按 Ready/Recording/Processing/Cancelling 分别解释为 Start/Stop/Cancel/NoOp；显式 Cancel 绑定窗口所见的 run，Ready 时只有 `targetRunId=None` 是当前目标。
- executor signal：`Progress(runId, payload)`、`Stopped(runId, terminal)`。
- `Invalid`：未知 intent、缺字段、无法反序列化的输入，或 stage 与 payload 不匹配的 signal。

判定先执行一个全局 admission 规则：窗口 Primary 的 `actionKey != derive(currentState)`，或显式 Cancel 的 `targetRunId != activeRun.runId` 时，返回 `NoOp` 与当前 snapshot，不进入 reducer；后端 hotkey adapter 在 Controller 串行 turn 内生成 Primary，天然使用当前动作。ActionKey 同时编码 expected action 与相关 run 身份，所以同一次双击或旧窗口 Primary 不会把 Stop 变成下一 stage 的 Cancel，也不会在旧 run 已结束后意外启动新 run；普通 progress 不改变 key，所以它不会误拒绝动作含义未变的 Primary。显式 Cancel 以 runId admission，因此同一 run 的 stage/mode 竞争会继续进入 arbiter，返回 Accepted 或 TooLate；旧 run 的 Cancel 不会取消新 run。通过 admission 的 intent 才进入下表。

`Progress.payload` 是按 stage 判定的穷尽 union，而不是自由组合的 `stage/status/payload` 字符串：`ContextCapture(ContextProgress)`、`RecordFinalize(StageProgress)`、`Preprocess(StageProgress)`、`Transcribe(TranscribeProgress)`、`Rewrite(RewriteProgress)`、`InsertPrepare(InsertPrepareProgress)`、`Finalize(Started)`。各可取消 stage progress 再区分 Started 与 Completed；只有 `TranscribeProgress::Completed` 携带 `TranscriptionResult`，只有 `RewriteProgress::Completed` 携带 `RewriteResult`，InsertPrepare 只携带已验证的目标/文本摘要，真正的 `InsertResult` 只存在于 Completed terminal。这样 reducer 可以同时穷尽匹配 payload 类型和第 3.1 节的 stage 顺序。流式 partial 和 audio level 仍是 display event，不进入 reducer。

`Stopped.terminal` 也是 outcome-discriminated typed union，而不是 option bag：

- `Completed { result: CompletedRunResult, warning? }` 必须携带 ASR、final text、timing 和 `InsertResult`。
- `Empty { timings }` 不能携带 result 或 error。
- `Failed { error: WorkflowError, recoveredResult?, recoveryErrors[] }` 必须携带决定本次 outcome 的 primary error，并可保留已经得到的 ASR/final text；恢复动作再失败时按发生顺序追加结构化 recovery error。
- `Cancelled { recoveredResult?, cleanupDiagnostic? }` 只携带允许恢复的中间文本和清理诊断，不携带成功 result。

Controller 投影 `lastRun` 时保持这三类诊断分离：`error` 是决定 outcome 的 primary error，`recoveryErrors` 只记录恢复动作自身的失败，`protocolContext? { receivedTerminal, receivedError? }` 只在 Controller 把非法 terminal 收敛为协议 Failed 时记录原 variant 与原 primary error。C4 不把被压过的 executor error 伪装成 recovery error；原 terminal 的 recovered result、timings 和 recoveryErrors 仍按各自字段保留。

非法字段组合在 signal 边界直接成为 Invalid。进程继续服务时，每个已接受 Primary/Start 必须恰好产生一个 `Stopped`；completion guard 只允许 executor 正常 terminal 或 supervisor 合成的 terminal 中一个胜出。资源无法证明释放而进入 fatal shutdown 时不伪造 `Stopped`，这是唯一明确例外。

判定严格按以下顺序执行，所以表内条件互斥：先验证 envelope 与 intent/signal 类型，未知或畸形输入成为 Invalid；合法窗口 Primary/Cancel 再分别做 actionKey/targetRunId admission；executor signal 再判断 `runId == activeRun.runId`；匹配的 Progress 校验 stage 顺序，匹配的 `Stopped` 最后按 outcome 分类。Primary 的 Cancel 分支与显式 Cancel 都先由该 run 的 executor arbiter 仲裁为 `Accepted | TooLate`，Controller 再选择对应规则。该仲裁是 outer handle 上的本地全函数，不是可能返回第三种 transport error 的 channel RPC；inner task 或控制通道已退出时，supervisor 必须先按既有 arbiter winner 收敛，而不是无条件领取 Failed。状态转移和 `revision` 提交都在 Controller 的串行上下文完成，I/O effect 在 reducer 外由 executor 执行。

### 4.2 转移表

表中“同态”表示不改变业务状态；admission NoOp、重复取消和晚到 signal 不增加 revision，也不重复广播 snapshot。每个规则都映射到第 8 节至少一个自动化场景。

| 规则 | 当前状态 | 互斥且穷尽的输入条件 | 目标状态 | 状态 owner 的原子提交 | effect/对外行为 | 场景 |
| --- | --- | --- | --- | --- | --- | --- |
| A1 | Any | Primary 的 actionKey 不等于当前派生值，或 Cancel 的 targetRunId 不匹配当前 activeRun.runId | 同态 | 无 | 返回 `NoOp` 与当前 snapshot；不进入 reducer、不广播 | T03, T04, T15 |
| R1 | Ready | Primary | Recording | 从 cached config 生成纯 seed，创建 runId，按 plan 初始化 context Pending/Skipped 并设置 dormant activeRun，revision + 1 | 解释为 Start；严格按 commit→Begin/BeginAccepted→非阻塞 snapshot enqueue→reply 执行，实际采集须在 Primary 后 200ms 内开始；失败最终按 signal 到达时走 G7/P8 | T01, T02, T03, T17, T19 |
| R2 | Ready | Cancel | Ready | 无 | 返回 `NoOp` 与当前 snapshot；不广播 | T03 |
| R3 | Ready | 任意 Progress 或 Stopped | Ready | 无 | 记录 `late_signal_ignored`；不写 History、不触发插入、不广播 | T16 |
| R4 | Ready | Invalid | Ready | 无 | 返回结构化 `E_WORKFLOW_INTENT_INVALID` | T03 |
| G1 | Recording | Primary | Processing | 保留同一 activeRun；按 context 子状态设置 ContextCapture Pending/Running 或 RecordFinalize Pending，revision + 1 | 解释为 Stop；提交后投递 Stop 并广播，投递失败由 supervisor 走 P8，UI 在 200ms 内进入 Processing | T03, T04 |
| G2 | Recording | Cancel，executor 返回 Accepted | Cancelling | 保留同一 activeRun/stage，revision + 1 | token 已由仲裁原子设置；广播 Cancelling snapshot | T03, T13, T15 |
| G3 | Recording | Cancel，executor 返回 TooLate | Recording | 无 | 返回 `CancelTooLate` 与当前 snapshot；等待唯一 Stopped | T03, T15 |
| G4 | Recording | 匹配 runId 且顺序合法的 ContextCapture Progress | Recording | 更新 context 子状态/elapsed，revision + 1 | 广播完整 snapshot；不执行持久化 | T05 |
| G5 | Recording | 匹配 runId 的 Stopped::Completed | Ready(Failed) | 移除 activeRun，保存 recovered result 与 `E_EXECUTOR_TERMINAL_ORDER`，revision + 1 | 资源已释放但 Recording 不允许成功终态；广播协议失败 | T06 |
| G6 | Recording | 匹配 runId 的 Stopped::Empty | Ready(Failed) | 移除 activeRun，保存 `E_EXECUTOR_TERMINAL_ORDER`，revision + 1 | 资源已释放但 Empty 只能从 Transcribe 终止；广播协议失败 | T07 |
| G7 | Recording | 匹配 runId 的 Stopped::Failed | Ready(Failed) | 移除 activeRun，保存 recoveredResult/primary/recovery errors，revision + 1 | 覆盖 Begin/录音/streaming/capture 失败、cleanup 升级和 supervisor 失败；广播诊断终态 | T02, T08, T21 |
| G8 | Recording | 匹配 runId 的 Stopped::Cancelled | Ready(Failed) | 移除 activeRun，保存 recoveredResult 与 `E_EXECUTOR_CANCEL_UNACKNOWLEDGED`，revision + 1 | 资源已释放但违反 Accepted→Cancelling 顺序；广播协议失败终态 | T13 |
| G9 | Recording | 不匹配 runId 的 Progress 或 Stopped | Recording | 无 | 记录 late signal 并忽略所有副作用 | T16 |
| G10 | Recording | Invalid | Recording | 无 | 记录结构化协议错误；录音资源不变 | T03, T05 |
| P1 | Processing | Primary 或 Cancel，executor 返回 Accepted | Cancelling | 保留 activeRun/stage，revision + 1 | Primary 解释为 Cancel；token 已由仲裁原子设置，所有可取消 stage 共用此规则 | T03, T14, T15 |
| P2 | Processing | Primary 或 Cancel，executor 返回 TooLate | Processing | 无 | 返回 `CancelTooLate`；terminal/finalization claim 已线性化，等待唯一 Stopped | T03, T15, T18 |
| P3 | Processing | 匹配 runId 且符合 plan 严格下一项或 Rewrite 失败恢复旁支的 Progress | Processing | 更新 stage/status/elapsed 和该 variant 的中间 result，revision + 1 | Context Completed 前移到 RecordFinalize Pending；Finalize::Started 使 cancelEnabled=false；广播完整 snapshot | T05, T09, T10, T11 |
| P4 | Processing | Finalize 中匹配 runId 的 Stopped::Completed | Ready(Completed) | 移除 activeRun，保存必备 CompletedRunResult/warning，revision + 1 | 唯一 History、必做 copy 和可选 auto-paste 已完成；广播终态 | T06, T09, T10, T12, T18 |
| P5 | Processing | Finalize 前匹配 runId 的 Stopped::Completed | Ready(Failed) | 移除 activeRun，保存 recovered result 与 `E_EXECUTOR_TERMINAL_ORDER`，revision + 1 | 资源已释放但成功终态过早；广播协议失败 | T06 |
| P6 | Processing | Transcribe 中匹配 runId 的 Stopped::Empty | Ready(Empty) | 移除 activeRun，保存 timings/Empty outcome，revision + 1 | 不进入 rewrite/InsertPrepare/Finalize；广播终态 | T07 |
| P7 | Processing | 非 Transcribe 中匹配 runId 的 Stopped::Empty | Ready(Failed) | 移除 activeRun，保存 `E_EXECUTOR_TERMINAL_ORDER`，revision + 1 | 资源已释放但 Empty 顺序非法；广播协议失败 | T07 |
| P8 | Processing | 匹配 runId 的 Stopped::Failed | Ready(Failed) | 移除 activeRun，保存 recoveredResult/primary/recovery errors，revision + 1 | 保留可用 ASR；广播结构化诊断 | T04, T08, T11, T18, T21 |
| P9 | Processing | 匹配 runId 的 Stopped::Cancelled | Ready(Failed) | 移除 activeRun，保存 recoveredResult 与 `E_EXECUTOR_CANCEL_UNACKNOWLEDGED`，revision + 1 | 资源已释放但违反 Accepted→Cancelling 顺序；广播协议失败终态 | T14 |
| P10 | Processing | 不匹配 runId 的 Progress 或 Stopped | Processing | 无 | 记录 late signal；不写 History、不插入、不广播 | T16 |
| P11 | Processing | Invalid | Processing | 无 | 记录结构化协议错误；executor 不变 | T03, T05 |
| C1 | Cancelling | Primary 或 Cancel | Cancelling | 无 | 返回 `NoOp` 与当前 snapshot；不重复发取消 | T03, T13, T15 |
| C2 | Cancelling | 匹配 runId 的 Progress | Cancelling | 无 | 忽略取消后的普通进度；UI 保持 Cancelling | T15 |
| C3 | Cancelling | 匹配 runId 的 Stopped::Cancelled | Ready(Cancelled) | 移除 activeRun，保留 recoveredResult/cleanupDiagnostic，revision + 1 | `Stopped` 已证明资源释放；广播一次取消终态 | T13, T14, T15, T20, T21 |
| C4 | Cancelling | 匹配 runId 的 Stopped::Completed、Empty 或 Failed | Ready(Failed) | 移除 activeRun，以 `E_EXECUTOR_TERMINAL_AFTER_CANCEL` 为 primary error；原 variant/error 进入 protocolContext，原 result/timings/recoveryErrors 保持各自语义；revision + 1 | 暴露违反 Accepted winner 的协议错误，不改写成 Cancelled | T15, T18 |
| C5 | Cancelling | 不匹配 runId 的 Progress 或 Stopped | Cancelling | 无 | 记录 late signal 并忽略 | T16 |
| C6 | Cancelling | Invalid | Cancelling | 无 | 记录结构化协议错误；继续等待当前 executor | T03 |

Cancel 与 resource/stage launch、普通终态、History/插入 finalization 不能只按 Controller mailbox 到达顺序判断，因为 executor 可能已经在另一个 task 中推进。每个 RunExecutor 因而用自己的串行 arbiter 处理 Begin resource registration、`request_cancel`、每次 `begin_stage`、`begin_terminal` 与 `begin_finalization`。Cancel 先于下一 launch/terminal claim 时原子设置 token 并阻止它；launch 已先开始时 Cancel 仍返回 Accepted，并立即通过同一 token/drop 终止它；只有 terminal/finalization claim 已先线性化时才返回 TooLate。Accepted reply 必须先于 completion future resolve，Controller 又在处理下一个 run signal 前提交 Cancelling；因此 Cancelled 只能到达 Cancelling，其他状态或 Cancelling 中的其他 terminal variant 都是可终止但必须暴露的 executor 协议错误。

竞争结果只有三种：匹配 `Stopped` 已先提交时，携带旧 targetRunId 的后到显式 Cancel 按 A1 no-op；Cancel 获得 Accepted 时，Controller 按 G2/P1 进入 Cancelling，executor 只按 C3 交付 Cancelled；`begin_terminal` 或 `begin_finalization` 先获得时，同一 run 的 Cancel 按 G3/P2 返回 TooLate，随后按 claim 对应的 typed terminal 结束。若 executor 违反 winner 并在 Cancelling 交付其他 variant，C4 以协议 Failed 终止；已经越过 finalization gate 的 History/平台输入不会被误标为 Cancelled，也不引入 Starting、Committing 等第五态。

## 5. 单次运行、错误和副作用契约

1. **一个 run 只有一个业务 ID。** Controller 在 Ready 接受 Primary/Start 时生成 `runId`，并把它传给录音、provider、History、trace、metric 和 UI。`transcriptId/taskId` 在目标协议中统一为 `runId`；录音 session ID 只作为 executor 内部资源 ID。
2. **一个 active run 只有一个 executor。** Ready 状态不保留 executor；R1 可以在本地暂存尚未 Begin 的 dormant handle，但必须随 Recording 原子转移所有权。Recording、Processing、Cancelling 始终持有同一 handle，Controller 又只在收到它的唯一 `Stopped` 后回到 Ready，所以不存在未被状态表示的外部资源，也不会让新 run 与旧资源重叠。
3. **中间结果不是持久化提交点。** ASR、rewrite 和 InsertPrepare capability 只返回 typed result，不自行改 workflow 或 History。Executor 与 Controller 可以保存当前 run 的 ASR/rewrite result 用于后续 stage 和 UI，但 History 写入延迟到 terminal finalization；当前 `rewrite` 与 `voice_workflow` 的重复更新不迁移。
4. **一个 run 只有一个不可逆 finalization gate。** Executor 串行仲裁 Cancel、resource/stage launch、`begin_terminal` 与 `begin_finalization`。Finalization gate 获得后先通过幂等 History port 写一次包含 ASR、final text 和 timing 的 run record；非空成功流程随后必须复制到剪贴板一次，再按 `auto_paste_enabled` 决定是否调用平台自动输入。History 失败不开始复制，复制失败产生 typed Failed，只有 auto-paste 失败且 copy 成功才是 Completed + warning。Empty 或 gate 前取消不写 History、不复制。任一 terminal claim 获得后 Cancel 返回 TooLate，所以不会出现 winner 与 typed terminal 不一致。
5. **错误携带已经获得的数据。** Preprocess/ASR 失败可以没有 recoveredResult；LLM、History 或 copy 失败必须用 `Stopped::Failed { error, recoveredResult, recoveryErrors }` 保留已经得到的 ASR/final text。Rewrite 失败若未被 Cancel 抢先，就从 Rewrite 进入有界 finalization，把 ASR 作为该 run 唯一 History 记录后结束且不自动复制；若这次恢复性 History 写入也失败，`error` 保留原 Rewrite error，History error 追加到 `recoveryErrors`，Record 页必须明确提示未能确认保存，并从 recoveredResult 展示 ASR 与直接复制入口。这个用户触发的恢复复制是对已投影结果的读/导出，不重新进入 executor，也不掩盖 History 失败。自动粘贴失败但剪贴板复制成功始终用 `Stopped::Completed` 的 warning 表达。
6. **取消不是丢弃 handle。** `request_cancel` 返回 Accepted 时 token 已设置，Controller 才进入 Cancelling。Accepted 阻止后续 resource/stage/terminal/finalization claim；已启动的 FFmpeg、HTTP/LLM future 和 streaming session 必须响应 token、abort/drop 并进入 cleanup，最终只能发 `Stopped::Cancelled`。普通 late result 不得再触发持久化或平台输入。
7. **执行和清理都必须有界且 fail closed。** 每个 provider/History/platform effect 都有明确且可测试的 operation deadline，超时进入 terminal/cleanup；用户主动保持的 Recording/streaming 以 heartbeat/idle 和 Stop/Cancel 后终止 deadline 约束，不用固定总时长误杀长录音。Finalization 不能因 History/copy 卡住而无限保持 TooLate。每个 resource port 还实现 `shutdown(deadline)` 与明确的 `force_abort/kill_and_wait`。Supervisor 先 cooperative shutdown；deadline 到期就记录 `run.cleanup_escalated` 并强制 abort/drop 或终止 FFmpeg 后 wait。强制路径成功释放资源时，Cancel winner 产生带 cleanupDiagnostic 的 Cancelled，其他 winner 产生 Failed；若 adapter 仍无法证明资源已经释放，则写 `run.cleanup_fatal` 并触发明确的进程 fatal shutdown/需重启路径，不能回 Ready、不能接受新 run，也不能伪造 `Stopped`。取消的整个清理预算仍受 300ms 上限约束，其他 stage/异常路径也必须使用有限 deadline。
8. **异常退出也必须服从既有 winner。** Inner task panic、control channel close 或 effect future 异常退出时，wrapper 先读取同一 arbiter：尚无 winner 时才调用 `begin_terminal(Failed)`；ordinary/finalization 已胜出时沿该所有权收敛为对应 Failed；Cancel 已胜出时异常只追加 cleanupDiagnostic，有界清理后仍必须发 Cancelled。这样异常退出不会把 Accepted cancel 改写成 Failed，也不会留下永久 Recording/Processing 或与正常结果产生两个终态；唯一例外是上条明确的 fatal shutdown，此时状态机不再继续服务。
9. **前端断开不影响业务终态。** R1 的 Begin 在事件 enqueue 之前发生，后续所有结果也先由 Controller 提交；事件 sink 失败只记诊断，不能阻止 executor。重新连接的窗口先注册 listener，再用 `workflow_snapshot` 恢复完整最新状态。

## 6. 前后端和事件契约

目标 Tauri 业务面只保留：

- `workflow_snapshot() -> WorkflowView`
- `workflow_command(Primary { actionKey } | Cancel { targetRunId? }) -> WorkflowCommandReply`

`WorkflowCommandReply` 包含完整 `view` 和 `disposition: Applied | NoOp | CancelTooLate`；Invalid 仍返回结构化 error。`CancelTooLate` 是命令仲裁结果，不进入持久状态，也不增加 revision。Primary 的业务含义只在 Controller 串行 turn 内决定；窗口只回传 opaque actionKey，不按本地 phase 把它改写成 Start/Stop/Cancel，hotkey 也不再先广播给 Overlay。显式 Cancel 保留，因为 Recording 的 Stop 与放弃 run 是两个真实用户意图。相同/旧 actionKey 的 Primary 按 A1 no-op，因此旧窗口和同一动作合同内的双击不会跨 stage 反转动作；普通 progress 后相同 key 仍然有效。显式 Cancel 用 targetRunId admission：同一 run 已从 Recording 进入 Processing 或 Finalize 时仍由 arbiter 返回 Accepted/TooLate，旧 run 请求则 NoOp。Hotkey adapter 必须保持“一次完整按下/释放只发一个 Primary”的边沿语义，过滤自动重复和重复 callback，再把该单一动作直接交给 Controller；两个独立物理手势仍分别按各自到达时的当前状态解释。

`WorkflowView` 至少包含 `revision`、派生的 `actionKey`、`mode`、`runId?`、`stage?`、`lastRun?`、`primaryLabel`、`primaryDisabled`、`cancelEnabled` 和完整诊断链；Failed lastRun 分别暴露 primary error、recoveryErrors 与可选 protocolContext。每个窗口挂载或重连时把 `latestRevision` 初始化为 None，先注册 `workflow.state` listener，再请求 snapshot；None 时第一份合法 view 无论 revision 是否为 0 都接受，之后只应用 `revision > latest`，忽略 `revision <= latest`。若 listener 的新事件先于 snapshot 到达，旧/相等 snapshot 自然被忽略，不会覆盖新状态。命令 reply 的 disposition 始终处理，但其中 view 也按相同 None/greater 规则替换，相等 revision 本来就是同一已提交状态。未知 mode 或缺少必填字段必须 fail closed，不能像当前 `workflowPhaseName` 一样降级成可 Start 的 Idle。

状态显示只使用 `workflow.state` 完整 snapshot。stage timing、audio level 和 partial text 可以作为带 `runId` 的 display event；窗口只显示 `runId == view.runId` 的 transient event，旧 run partial 直接丢弃。它们没有 `stateChanging` effect，不能回流 Controller。R1 的跨边界全序固定为 state/revision commit → executor Begin/BeginAccepted → nonblocking `workflow.state` enqueue → command reply；Begin 或 sink 失败都不能改写这个顺序，前者最终走唯一 Failed terminal，后者只记显示诊断。终态 toast、History refresh 和 Overlay final text 都从新 revision 的 `lastRun` 派生，不再分别消费 `transcription.completed`、`rewrite.completed`、`insertion.completed` 后执行业务命令。

## 7. 最小观测契约

统一 debug 因果链只需要两个关联/排序字段：

- `runId`：一次用户录音从 Primary/Start 到 `Stopped` 的业务关联 ID；所有 stage、错误、History row 和资源 trace 都必须携带。
- `revision`：Controller 每次实际提交的单调版本；`workflow.transition` trace 和 `workflow.state` snapshot 使用同一值。

`actionKey` 是当前状态的纯派生 admission 值，不是计数器或业务 ID；命令诊断可以记录它，但它不形成第三条因果链。

每次真实转移写一条 `workflow.transition`：`runId, revision, from, to, cause, stage?, outcome?, errorCode?, recoveryErrorCodes?, protocolTerminal?, protocolErrorCode?`。后两个字段只用于 C4，不能混入 recoveryErrorCodes。Executor 在 ordinary/finalization gate 首次决出 winner 时分别写 `run.terminal_won` 或 `run.finalization_won`，每个 Cancel 请求另写 `run.cancel_arbitrated` 与 disposition；cleanup 超时/强制成功/无法证明释放分别写 `run.cleanup_escalated`、`run.cleanup_forced`、`run.cleanup_fatal`。这样 TooLate、Cancelled、双错误与 fatal containment 都能由同一 runId 证明。其余记录继续使用现有 `stage/step_id/op/status/duration_ms`，但 `task_id` 统一写 runId。资源 ID、provider、设置摘要 hash 和 intent source 放在 trace context；它们不能参与状态匹配。文本、音频、API key 和完整路径仍按现有脱敏与 debug opt-in 规则处理。

排障路径因而固定为：用户动作生成/携带 runId -> 查 `workflow.transition` 确认最后 revision 和 cause -> 按同一 runId 查 executor stage/step -> 对照唯一 `Stopped` 与 `lastRun`。不再需要把 UI eventId、两个窗口的本地 ref 或录音 session ID 拼成第二条因果链。

## 8. 自动化验收模型

本节定义的测试已经实现为可执行合同。最小测试缝只有四个：可脚本化 dormant `RunHandle`，其中含 resource/stage/cancel/terminal/finalization arbiter、completion guard 和受控 supervisor exit；带调用计数的 ASR/rewrite/insertion/History ports；捕获 snapshot/事件的 sink；确定性的 ID、clock 和 200/300ms deadline。状态测试不需要 Tauri、真实网络或真实硬件；T23 另要求 Windows/Linux 平台 adapter gate，未执行的目标 gate 不能记为 PASS。

### 8.1 不变量

- I1：只有 Controller reducer 能改变 mode、activeRun、lastRun 和 revision；actionKey 只能由这些已提交字段纯派生。
- I2：Ready 没有 executor 或活跃外部资源；其余三态恰有一个相同 runId 的 handle。Dormant handle 必须先随 R1 提交，再启动外部资源。
- I3：进程继续服务时，每个已接受 Primary/Start 恰好一个 typed `Stopped`、一个终态 snapshot 和最多一次 History finalization commit；cleanup 先有界 graceful、再 force，无法证明释放则 fatal shutdown 而不是 Ready。
- I4：所有可接受 input 都命中第 4 节恰好一条规则；stale actionKey 或 mismatched targetRunId 只命中 A1，Invalid 只返回结构化错误且不变状态。
- I5：mismatched/late signal 不改状态、不增 revision、不写存储、不执行插入。
- I6：状态先原子提交，再发对应 revision 的完整 snapshot；前端没有 report/apply 命令。
- I7：Begin resource 与每个可取消 stage launch、`begin_terminal`、`begin_finalization`、Cancel 都经同一 arbiter；Accepted 后不再启动新工作或领取 terminal ownership，已经启动的请求响应 token/drop。
- I8：Cancel、ordinary terminal 与 finalization 只有一个 winner：Cancel winner 只能发 Cancelled，其他 claim winner 使 Cancel TooLate；History/复制已提交的 run 不能终止为 Cancelled。
- I9：LLM 或后续失败保留 ASR，恢复失败追加 recoveryErrors，音频始终清理；每个非空 Completed 必有一次 copy，只有 auto-paste 可选；Windows platform input、Linux AT-SPI 与 macOS Core Graphics 都不得用快捷键模拟。
- I10：Progress 和 Stopped 都是 typed union；stage/terminal 顺序非法不能覆盖中间结果，资源已释放的非法 terminal 必须收敛为结构化 Failed，不能被 Controller 改写成另一 variant。
- I11：每个窗口的 latestRevision 从 None 开始，listener 先于 snapshot；首份 revision 0、重连乱序 snapshot 和相等 command reply 都不会被误丢或回退。
- I12：窗口 Primary 原样回传派生 actionKey，显式 Cancel 携带 targetRunId，后端热键的一次完整物理手势只发送一个 Controller 内部 Primary；动作映射不读取窗口 phase，同 key 重复 Primary 不跨 stage 生效，普通 progress 不使 key 过期，同一 run 的 Cancel 必须得到 Accepted/TooLate 而不是 stale projection NoOp。

### 8.2 场景

| ID / 测试名 | 输入与受控异步结果 | 预期状态、事件和副作用 | 最小测试缝 |
| --- | --- | --- | --- |
| T01 `start_freezes_seed_and_orders_begin_projection` | Ready + fresh Primary；cached full seed 与 dormant handle；Begin 选择 Doubao；R1 后修改 settings | R1 全序为 commit→实际音频采集及录音/streaming handle 注册→BeginAccepted→snapshot enqueue→reply；Primary 到采集开始 <=200ms，当前 run 使用旧 seed、下一 run 才用新值，capture I/O 全在 commit 后 | ordered handle、config spy、I/O probe、clock、sink |
| T02 `begin_or_recording_start_failure_uses_typed_failed` | 分别让 Begin delivery/ack、录音/Doubao session、ContextCapture 启动失败，并让 Context 失败发生在 Primary Stop 前/后 | R1 后统一 G7/P8：cleanup 后唯一 Stopped::Failed 带同一 runId/error；没有 Start/capture 专用终态通道 | scripted handle、resource counters |
| T03 `intent_admission_and_matrix_are_total` | 四态遍历 fresh Primary/Cancel/Invalid、stale actionKey 和 mismatched targetRunId；验证 Initial/After(lastRun.runId) 的两代 Ready key；active cancel 脚本化 Accepted/TooLate；hotkey 注入重复 keydown/callback 与两次完整手势 | 精确命中 A1、R1/R2/R4、G1-G3/G10、P1/P2/P11、C1/C6；旧 Ready key 不能在后续 Ready 启动新 run；有效 intent 各有 disposition，Invalid 各有结构化 error；NoOp/TooLate 不增 revision；每次完整热键手势至多一个 Primary | reducer harness、arbiter、hotkey edge fake、handle recorder |
| T04 `stop_is_once_and_delivery_failure_is_terminal` | Recording 同一 actionKey 的两次 Primary；另在 view 与 Primary 之间提交 G4 progress，并让一例 Stop 投递失败 | 双击命中 G1/A1 且 Stop control 一次；只有 progress 变新时旧 view 的相同 actionKey 仍命中 G1；两者 200ms 内 Processing。投递失败仍先 G1，随后唯一 P8 Failed，不永久卡住 | handle recorder、clock、supervisor |
| T05 `typed_progress_domain_is_strict_and_monotonic` | Primary Stop 分别早于 Context Started、介于 Started/Completed、晚于 Completed；遍历完整 plan 与 Rewrite 失败分支；注入重复/回退/错 payload | G1 按 context 子状态选择首 stage，合法 G4/P3 永不回退；非法 G10/P11、E_EXECUTOR_PROGRESS_ORDER 不增 revision或覆盖 result | typed fixtures、plan variants、sink |
| T06 `completed_terminal_is_typed_and_ordered` | 构造必带 result 的 Stopped::Completed；分别从 Finalize、Recording、Finalize 前发送，并尝试缺 result 反序列化 | Finalize 走 P4；Recording 走 G5、过早 Processing 走 P5，二者为 E_EXECUTOR_TERMINAL_ORDER；缺 result 被 typed boundary 拒绝 | terminal fixtures、serde/compile contract、sink |
| T07 `empty_terminal_is_typed_and_ordered` | 构造无 result/error 的 Stopped::Empty；分别从 Transcribe、Recording、其他 Processing stage 发送，并尝试附带 error | Transcribe 走 P6 且不写 History；Recording G6、错 stage P7 收敛协议 Failed；非法字段被 boundary 拒绝 | terminal fixtures、port counters |
| T08 `failure_timeout_or_abnormal_exit_is_terminal_once` | Context/录音/provider 与每个 processing/finalization effect 分别 error/timeout；inner panic/channel close；控制正常或 supervisor 赢 guard | 失败 winner 走 G7/P8，正常 winner 走 P4；进程继续时恰好一个 typed Stopped/终态，loser 不能再提交，Finalize 不会无限 TooLate | parameterized ports、deadline、supervisor exit、completion guard |
| T09 `plan_without_rewrite_still_copies_once` | rewrite=false、auto_paste=false；ASR、History、copy 成功 | P3/P4：rewrite 0、History 1、copy 1、auto-paste 0；InsertPrepare 与 Finalize 顺序可追踪 | plan seed、port counters |
| T10 `plan_with_rewrite_and_autopaste_runs_once` | rewrite=true、auto_paste=true；ASR/rewrite/History/copy/paste 成功 | P3/P4：final text 来自 rewrite；History/copy/paste 和每个 capability 各 1 | scripted ports、trace sink |
| T11 `rewrite_failure_preserves_asr_and_double_faults` | ASR 成功、rewrite 失败、finalization 胜出；分别让恢复性 History 成功/失败，并触发失败结果的手动复制 | P3/P8：从 Rewrite 合法进 Finalize；成功时唯一 record 保存 ASR，失败时 primary error 仍是 Rewrite、History 进入 recoveryErrors，Record failure projection 标注未保存，手动导出把 recoveredResult 复制成功；executor automatic copy/paste 仍为 0 | History/rewrite fake、clipboard exporter、projection/diagnostic assertions |
| T12 `paste_warning_keeps_completed_copy` | copy=true、autoPasteAttempted=true、autoPasteOk=false | P4：typed Completed 带 InsertResult/warning，文本、History 和复制成功不丢失 | insertion fake、sink |
| T13 `cancel_recording_resources_and_context_are_serial` | 录音/Doubao/Context resource launch 与显式 Cancel 两种顺序；Accepted 后延迟 cleanup，并分别触发 inner panic/control close；另直接注入 Recording Cancelled | launch 先则 token 取消、Cancel 先则后续 launch 拒绝；异常只追加 cleanupDiagnostic，G2/C1/C3 仍在 300ms 内 Ready(Cancelled)；直接 terminal 走 G8 协议 Failed | controllable arbiter、supervisor exit、clock、resource counters |
| T14 `cancel_each_processing_stage_and_reject_direct_terminal` | Stop 后 Context Pending/Running 与其后五个 Processing stage，各在 launch 前/后用 Primary/Cancel；另直接注入 Cancelled | 六个 boundary 都走 P1/C3：Accepted 后无新 launch，已启动请求收 token；直接 terminal 走 P9 协议 Failed，History/copy 0 | parameterized executor、deadline |
| T15 `cancel_terminal_and_finalization_are_linearizable` | Stopped→旧 targetRunId Cancel；同 run Recording→Processing 或 Finalize::Started 与显式 Cancel 竞争；Cancel/Accepted→重复 intent/Progress→Cancelled；begin_terminal/finalization→Cancel/TooLate→terminal；Accepted 后注入非 Cancelled | 旧 run Cancel 命中 A1；跨 mode/stage 的同 run Cancel 仍命中 G2/P1 Accepted 或 P2 TooLate；Accepted 后走 C1/C2/C3，claim 先胜出后走 G3/P2+G7/P4/P6/P8；违规 variant 走 C4。每种只发布一个终态 | ordered arbiter/mailbox harness |
| T16 `late_or_duplicate_signal_has_no_effect` | Ready/active/Cancelling 收旧 runId 或重复 terminal；窗口收到旧 run streaming partial | R3/G9/P10/C5：状态、revision、History、copy、snapshot 均不变；旧 partial 不显示 | two run IDs、all counters、projection harness |
| T17 `new_run_after_any_outcome_gets_new_identity` | Completed/Empty/Failed/Cancelled Ready 后 fresh Primary | 每次 R1 且 runId 不复用；lastRun 可追溯但不成为 active | deterministic ID source |
| T18 `finalization_gate_excludes_partial_or_cancelled_commit` | Cancel/finalization 分别先拿 gate；重复 callback；History 失败；Accepted 后逐一注入 Completed/Empty/Failed，其中 Failed 自带 primary/recovery errors | Cancel 胜出 C3 且 History/copy=0；finalization 胜出 P2 后只走 P4/P8；History 最多 1、成功后 copy 最多 1；违规 terminal 走 C4，以协议错误为 primary、原 variant/error 进 protocolContext，原 recoveryErrors 不改义 | serial arbiter、effect ports、fault injection、diagnostic assertions |
| T19 `projection_bootstrap_and_revision_order_are_total` | 捕获 R1 commit/Begin/sink/reply；latest=None 时先 listener，再排列 rev0、新事件、重连 snapshot、相等 reply | R1 先 Begin 再投影；首份合法 view 总接受，之后只应用 `>`、忽略 `<=`，disposition 仍处理；无 report/apply | state spy、window contract harness |
| T20 `ready_implies_no_live_run_resources` | 对成功、失败、空结果、取消遍历 terminal | P4/P6/P8/C3 每次进入 Ready 时 FFmpeg/request/token/temp asset/handle 计数均为 0 | resource-owning fake executor |
| T21 `cleanup_timeout_escalates_or_fails_closed` | cooperative cleanup 卡住；分别让 force kill/abort 成功或仍无法证明释放 | force 成功按 winner 走 G7/P8 或 C3 且资源为 0；无法证明时 fatal，不产生 Ready/新 run/伪 Stopped | hung resource fake、deadline、fatal hook |
| T22 `verify_runs_nonzero_workspace_contracts` | 执行 quick/full test selection，注入 0-match 和 engine sentinel | 0-match 必须失败；full 必须执行 workspace engine 与前端契约 | xtask process harness/sentinel |
| T23 `insertion_ports_are_cross_platform_and_real` | 对同一文本运行 Windows platform-input 与 Linux AT-SPI adapter；关闭/开启 auto-paste，并注入 copy/paste 失败 | 两端都先 copy 且不用快捷键模拟；关闭时不输入，开启时各调用原生能力；各 OS gate 必须实际执行，未执行的平台不等于全局 PASS | shared port contract、Windows/Linux integration runners |

第 4 节每条规则都已映射到 T01-T21；T03 遍历 admission 与全部用户 intent 分支，T05 定义完整 progress 合法域，T06/T07 拒绝 typed terminal 的非法字段与顺序，T08/T21 覆盖异常退出和有界 cleanup，T13-T15 覆盖两态取消与 terminal winner。反向每个状态场景都引用 A/R/G/P/C 规则或本节不变量。T01-T23 已实现为可执行合同，T22 防止 crate 拆分后的 0-test 假通过；T23 仍要求两个平台 gate 各自实际执行，单个平台的 Skipped/NotRun 不能作为跨平台 PASS。

## 9. 已完成的最小迁移

```text
重构前：UI primary/report/auto-flow
        -> VoiceWorkflow 九 phase
        -> detached voice_tasks + 三个全局 active 槽
        -> UI terminal event -> UI report -> backend

当前：UI Primary(actionKey)|Cancel(targetRunId) admission / hotkey edge Primary
        -> WorkflowController 四态 + revision
        -> one RunExecutor(runId) -> typed Progress/Stopped
        -> Controller commit -> full snapshot -> UI projection
```

迁移已按以下依赖顺序完成：

1. 建立纯四态 reducer、Primary/Cancel admission、typed Progress/Stopped 和本文矩阵，用 fake executor 跑 T01-T21，再迁移 capability 代码。
2. 让 `workflow_command` 和 backend hotkey 只进入 Controller；Ready 的 Primary 从 cached config 创建完整 RunPlanSeed 与 dormant handle，随 Recording 原子提交后按 Begin→snapshot→reply 全序启动。三个全局 active 槽已改为 executor 持有的 run-scoped handle，并由 completion guard/supervisor 把所有可继续运行的退出收敛为恰好一个 Stopped。
3. 把 context capture、provider、rewrite、InsertPrepare、History 和平台输入组合进一次 run。能力模块不再自行推进 workflow 或写 History；Executor 用同一 arbiter 排序 resource/stage launch、Cancel、ordinary terminal 与 finalization，terminal gate 后只提交一条 History、一次必做 copy 和一次可选 auto-paste。
4. 后端先处理 typed progress、cancel disposition 和 typed Stopped，再发完整 snapshot；`workflow_apply_event`、`workflow_report_*`、`applied_event_views`、`insert_previous_phase` 和前端 ASR 回报已经删除。
5. 删除没有实时 UI 调用者且与冻结自动流程冲突的 `rewriteLast/insertLast/copyLast`、`record_transcribe_*`、`rewrite_text/insert_text` Tauri 兼容入口；History copy 保持独立。若未来增加手动重试，它必须作为新用户 intent 进入同一 Controller，并先扩展完整命令矩阵，不能恢复前端直调 capability。
6. 删除 Main 的 `autoRewriteStartedRef/autoInsertStartedRef` 和 phase 编排，删除 Overlay 的业务回报与 hotkey 转发；窗口 Primary 原样回传 actionKey、显式 Cancel 携带 targetRunId，hotkey adapter 过滤重复边沿后直接进入 Controller，并按 snapshot 渲染 label/disabled；当前 run partial 只作显示。
7. 将原有 `task_id` 语义统一为 runId，并在每次 reducer commit 写 `workflow.transition`；状态型 `effect/eventId` 回流协议已经删除，UiEventMailbox 只负责显示传输。
8. 实现有界 cleanup/force/fatal 与 Windows/Linux insertion adapter gate，修正 xtask 的 workspace/package 选择和 0-match 检查，接入 T21-T23，并通过 project-index 生成器刷新 AGENTS 自动索引。

这条迁移没有引入兼容 shim 或并行状态机。每一步都把一个旧所有者删除后再接入唯一边界，没有让新旧终态通道同时成为业务真相。

## 10. 成本下降的可观察结果

**维护：** 重构前修改 ASR 完成需要同时理解 `voice_tasks`、两个前端 listener、三个 report 命令和九 phase；当前实现把所有终态收敛为 `Stopped` 加一张 reducer 表。维护者新增错误或调整恢复行为时，只修改单次 executor 结果和一条 Controller 规则，T03/T06-T08 会直接证明矩阵仍完整。

**扩展：** 重构前新增处理步骤通常需要新增 phase、前端 auto ref、事件 kind 和 report 路径。当前实现中的新步骤默认只是 RunExecutor stage，只要它服从同一 token 和 `Stopped` 契约，就不改变业务状态或 UI 命令；只有真的产生不同用户动作或资源寿命时才有理由增加状态。

**debug：** 重构前 task/transcript/session/event ID 和窗口本地状态分散，命令错误甚至缺 task ID。当前实现使一次录音只有 runId，每次真实转移只有一个 revision；从用户动作、FFmpeg、provider、History 到终态都能用同一查询串联，late signal 会留下明确的 ignored trace，而不是表现为偶发 UI toast 或卡住。

**可靠性：** 重构前前端断开会阻止 ASR 落状态，取消后资源释放与新 Start 也没有统一证明。当前实现先由后端提交、再投影 UI，并用 Cancelling 保持 executor handle 直到 `Stopped`；T13-T16、T20 能观察到“不会因窗口缺席卡住、不会由晚到结果覆盖、Ready 时没有旧资源”这三个直接后果。
