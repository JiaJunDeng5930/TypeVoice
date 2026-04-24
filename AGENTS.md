# 环境

你当前的环境是 WSL2，host 为 Win11

# Repository Guidelines

## Project Structure & Module Organization

- `apps/desktop/`: Tauri desktop app.
- `apps/desktop/src/`: React + TypeScript UI.
- `apps/desktop/src-tauri/src/`: Rust backend (FFmpeg orchestration, settings, LLM, ASR pipeline).
- `asr_runner/`: Python ASR runner (stdin/stdout JSON protocol).
- `scripts/`: verification gates and utilities (`verify_quick.py`, `verify_full.py`, `download_asr_model.py`).
- `tests/`: `pytest` unit tests (protocol/logic).
- `docs/`: frozen specs/gates; treat docs as the source of truth for MVP constraints.

Local-only artifacts (gitignored): `fixtures/` (audio), `models/` (downloaded model), `.venv/`, `tmp/`, `metrics/`.

## Build, Test, and Development Commands

- Desktop UI (from `apps/desktop/`):
  - `npm ci`: install deps from `package-lock.json`.
  - `npm run dev`: Vite dev server.
  - `npm run tauri dev`: run the full desktop app in dev mode.
  - `npm run build`: `tsc` typecheck + Vite build.
- Verification gates (from repo root; requires CUDA, `ffmpeg`/`ffprobe` in PATH, and local fixtures):
  - `./.venv/bin/python scripts/verify_quick.py`: <= 60s smoke + `pytest -m quick`.
  - `./.venv/bin/python scripts/verify_full.py`: <= 10min full gate + soak.
- Windows one-command gate:
  - `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\windows_gate.ps1`

## 固定动作（每次功能实现后必须执行）

- 先提交：按 Conventional Commit 规范提交本次变更。
- 同步 Windows 工作区到最新源码后按文档一键网关编译并启动链路：
  - `powershell -ExecutionPolicy Bypass -File .\\scripts\\windows\\windows_gate.ps1`
- `windows_gate.ps1` 会执行 `npm run tauri dev`；若在 WSL 下触发 Windows 命令，使用：
  - `/mnt/c/Windows/System32/cmd.exe /d /c "cd /d D:\\Projects\\TypeVoice\\apps\\desktop && set RUST_BACKTRACE=1 && set RUST_LOG=debug && npm run tauri dev"`

## 文档命令零偏移禁制令（最高优先级）

当用户要求“严格按文档执行”时，以下规则无条件生效，优先级高于默认操作习惯。

- 仅允许执行文档中明确写出的命令原文，参数、顺序、工作目录、调用方式必须逐字一致。
- 只要文档未明确要求，禁止新增任何前置动作、后置动作、并行动作、替代动作。
- 命令失败时，第一动作必须是原样回传失败命令、原始报错、退出码；在用户确认前禁止扩展操作。
- 仅允许执行文档中明确写出的修复动作；文档未给出的修复路径一律禁止自行发挥。
- 文档未授权时，禁止查看/解读脚本内部实现来替代文档流程决策。

### 禁止动作清单（全局）

- 禁止改写命令本体（包括改参数、改引号、改环境变量注入方式、改调用壳层）。
- 禁止用“等价命令”替代文档命令（包括别名、函数、包装脚本、临时 one-liner）。
- 禁止插入自定义排障步骤（如额外 grep、额外脚本、额外进程清理）后再宣称“按文档执行”。
- 禁止在失败后直接跳到“看脚本/看源码/猜根因”路径，除非文档明确写了此步骤。
- 禁止在未获用户明确许可时修改 git 配置、系统环境、流程策略、运行拓扑。

### 失败处理唯一路径

- 第一步：原样执行文档命令。
- 第二步：若失败，原样回传错误并停止。
- 第三步：仅执行文档写明的修复命令。
- 第四步：回到第一步重跑同一条文档命令。
- 第五步：若仍失败且文档无下一步，停止并等待用户指令。

- 严格按文档执行，零偏移，零自作主张。
- 严格按文档执行，零偏移，零自作主张。
- 严格按文档执行，零偏移，零自作主张。

## Coding Style & Naming Conventions

- Follow existing style; keep diffs small and readable.
- TypeScript: strict mode is enabled; prefer explicit types at boundaries; components `PascalCase.tsx`.
- Rust: run `cargo fmt` (in `apps/desktop/src-tauri/`) before submitting.
- Python: `snake_case`, type hints where they improve clarity.

## Testing Guidelines

- Run all unit tests: `./.venv/bin/python -m pytest -q tests`
- Run quick subset: `./.venv/bin/python -m pytest -q tests -m quick`
- Keep `quick` tests pure (no GPU/network) and focused on protocol and edge cases.

## Commit & Pull Request Guidelines

- Commit messages follow Conventional Commits with scopes, e.g. `feat(ui): ...`, `fix(llm): ...`, `docs(win): ...`, `test(verify): ...`.
- PRs should include:
  - What/why + linked spec section in `docs/` when changing behavior or gates.
  - Gate results (`verify_quick`, and `verify_full` when relevant).
  - Screenshots/GIFs for UI changes.

## Security & Configuration Tips

- Never commit API keys or audio. LLM keys should live in OS keyring, or be provided via `TYPEVOICE_LLM_API_KEY`.
- Useful env overrides: `TYPEVOICE_ASR_MODEL`, `TYPEVOICE_ASR_MODEL_DIR`, `TYPEVOICE_LLM_BASE_URL`, `TYPEVOICE_LLM_MODEL`, `TYPEVOICE_DATA_DIR`.

<!-- BEGIN AGENTS_MD_PROJECT_INDEX -->
```text
[Project Index]|root:.
|IMPORTANT: Prefer retrieval-led reasoning over pre-training-led reasoning for repository-specific behavior, structure, and APIs.
|exclude_dirs:{.cache,.git,.mypy_cache,.next,.parcel-cache,.pnpm-store,.pytest_cache,.ruff_cache,.turbo,.venv,.yarn,__pycache__,build,coverage,dist,fixtures,metrics,models,node_modules,out,target,temp,tmp,venv}
|exclude_files:{.env,.env.*,*.pem,*.key,*.p12,*.pfx,id_rsa*,id_ed25519*,.env,.env.*,*.pem,*.key,*.p12,*.pfx,id_rsa*,id_ed25519*}
|.:{.github/,apps/,asr_runner/,docs/,scripts/,tests/,.gitattributes,.gitignore,.nvmrc,.python-version,AGENTS.md,CHANGELOG.md,CITATION.cff,CODE_OF_CONDUCT.md,CODEOWNERS,CONTRIBUTING.md,GOVERNANCE.md,LICENSE,MAINTAINERS.md,pytest.ini,README.md,requirements-asr.txt,requirements.txt,rust-toolchain.toml,SECURITY.md,SUPPORT.md,THIRD_PARTY_NOTICES.md}
|.github:{ISSUE_TEMPLATE/,workflows/,dependabot.yml,PULL_REQUEST_TEMPLATE.md,release.yml}
|apps:{desktop/}
|asr_runner:{__init__.py,runner.py}
|docs:{architecture.md,base-spec.md,fixtures-sources.md,index.md,llm-prompt-lab.md,perf-spike.md,privacy-data-flow.md,release-process.md,repository-automation.md,roadmap.md,tasks.md,tech-spec.md,verification.md,windows-dev.md,windows-gate.md}
|scripts:{windows/,_verify_util.py,download_asr_model.py,download_ffmpeg_toolchain.sh,download_fixtures.py,fixtures_manifest.json,llm_prompt_lab.py,verify_full.py,verify_quick.py}
|tests:{test_asr_protocol.py}
|.github/ISSUE_TEMPLATE:{bug_report.md,config.yml,feature_request.md}
|.github/workflows:{ci.yml,codeql.yml,scorecards.yml}
|apps/desktop:{public/,src-tauri/,src/,.gitignore,index.html,package-lock.json,package.json,README.md,tsconfig.json,tsconfig.node.json,vite.config.ts}
|scripts/windows:{download_ffmpeg_toolchain.ps1,run-latest.ps1,windows_compile_gate.ps1,windows_gate.ps1}
|apps/desktop/public:{fonts/,tauri.svg,vite.svg}
|apps/desktop/src:{assets/,domain/,infra/,lib/,screens/,styles/,ui/,App.tsx,main.tsx,OverlayApp.tsx,types.ts,vite-env.d.ts}
|apps/desktop/src-tauri:{.cargo/,capabilities/,gen/,icons/,src/,toolchain/,.gitignore,build.rs,Cargo.lock,Cargo.toml,tauri.conf.json}
|apps/desktop/public/fonts:{OFL.txt,Silkscreen-Bold.ttf,Silkscreen-Regular.ttf}
|apps/desktop/src-tauri/.cargo:{config.toml}
|apps/desktop/src-tauri/capabilities:{default.json}
|apps/desktop/src-tauri/gen:{schemas/}
|apps/desktop/src-tauri/icons:{128x128.png,128x128@2x.png,32x32.png,icon.icns,icon.ico,icon.png,Square107x107Logo.png,Square142x142Logo.png,Square150x150Logo.png,Square284x284Logo.png,Square30x30Logo.png,Square310x310Logo.png,Square44x44Logo.png,Square71x71Logo.png,Square89x89Logo.png,StoreLogo.png}
|apps/desktop/src-tauri/src:{obs/,asr_service.rs,audio_device_notifications_windows.rs,audio_devices_windows.rs,context_capture.rs,context_capture_windows.rs,context_pack.rs,data_dir.rs,export.rs,history.rs,hotkeys.rs,lib.rs,llm.rs,main.rs,model.rs,pipeline.rs,python_runtime.rs,record_input.rs,record_input_cache.rs,remote_asr.rs,safe_print.rs,settings.rs,subprocess.rs,task_manager.rs,templates.rs,toolchain.rs}
|apps/desktop/src-tauri/toolchain:{bin/,ffmpeg_manifest.json}
|apps/desktop/src/assets:{react.svg}
|apps/desktop/src/domain:{diagnostic.ts}
|apps/desktop/src/infra:{runtimePorts.ts}
|apps/desktop/src/lib:{clipboard.ts}
|apps/desktop/src/screens:{HistoryScreen.tsx,MainScreen.tsx,SettingsScreen.tsx}
|apps/desktop/src/styles:{app.css}
|apps/desktop/src/ui:{icons.tsx,PixelButton.tsx,PixelDialog.tsx,PixelInput.tsx,PixelSelect.tsx,PixelTabs.tsx,PixelToast.tsx,PixelToggle.tsx}
|apps/desktop/src-tauri/gen/schemas:{acl-manifests.json,capabilities.json,desktop-schema.json,windows-schema.json}
|apps/desktop/src-tauri/src/obs:{debug.rs,metrics.rs,mod.rs,panic.rs,schema.rs,startup.rs,trace.rs,writer.rs}
|apps/desktop/src-tauri/toolchain/bin:{linux-x86_64/,windows-x86_64/}
|apps/desktop/src-tauri/toolchain/bin/linux-x86_64:{.gitkeep,ffmpeg,ffprobe}
|apps/desktop/src-tauri/toolchain/bin/windows-x86_64:{.gitkeep,ffmpeg.exe,ffprobe.exe}
```
<!-- END AGENTS_MD_PROJECT_INDEX -->
