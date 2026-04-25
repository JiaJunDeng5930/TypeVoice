use std::{
    env,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand, ValueEnum};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use typevoice_core::context_pack::{
    self, ContextBudget, ContextSnapshot, HistorySnippet, PrevWindowInfo,
};

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "TypeVoice repository tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Verify {
        #[command(subcommand)]
        command: VerifyCommand,
    },
    Fixtures {
        #[command(subcommand)]
        command: FixturesCommand,
    },
    Toolchain {
        #[command(subcommand)]
        command: ToolchainCommand,
    },
    Gate {
        #[command(subcommand)]
        command: GateCommand,
    },
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    LlmPromptLab(LlmPromptLabArgs),
}

#[derive(Subcommand)]
enum VerifyCommand {
    Quick,
    Full,
}

#[derive(Subcommand)]
enum FixturesCommand {
    Download,
}

#[derive(Subcommand)]
enum ToolchainCommand {
    Ffmpeg(ToolchainFfmpegArgs),
}

#[derive(Debug, Clone, Args)]
struct ToolchainFfmpegArgs {
    #[arg(long, value_enum, default_value_t = ToolchainPlatform::Auto)]
    platform: ToolchainPlatform,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ToolchainPlatform {
    Auto,
    All,
    #[value(name = "windows-x86_64")]
    WindowsX86_64,
    #[value(name = "linux-x86_64")]
    LinuxX86_64,
}

#[derive(Subcommand)]
enum GateCommand {
    Windows,
    WindowsCompile,
}

#[derive(Subcommand)]
enum RunCommand {
    Latest,
}

#[derive(Debug, Clone, Args)]
struct LlmPromptLabArgs {
    #[arg(long, default_value = "")]
    base_url: String,
    #[arg(long, default_value = "")]
    model: String,
    #[arg(long, default_value = "")]
    reasoning_effort: String,
    #[arg(long, default_value = "")]
    api_key: String,
    #[arg(long)]
    system_prompt_file: Option<PathBuf>,
    #[arg(long, default_value = "")]
    system_prompt: String,
    #[arg(long)]
    edit: bool,
    #[arg(long, default_value = "")]
    transcript: String,
    #[arg(long)]
    transcript_file: Option<PathBuf>,
    #[arg(long)]
    history_file: Option<PathBuf>,
    #[arg(long, default_value = "")]
    clipboard: String,
    #[arg(long)]
    clipboard_file: Option<PathBuf>,
    #[arg(long, default_value = "")]
    prev_title: String,
    #[arg(long, default_value = "")]
    prev_process: String,
    #[arg(long, value_enum, default_value_t = InjectMode::InlineOneUser)]
    inject_mode: InjectMode,
    #[arg(long, default_value_t = 3)]
    max_history_items: usize,
    #[arg(long, default_value_t = 600)]
    max_chars_per_history: usize,
    #[arg(long, default_value_t = 800)]
    max_chars_clipboard: usize,
    #[arg(long, default_value_t = 60.0)]
    timeout_s: f64,
    #[arg(long, default_value = "")]
    out_dir: String,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    print_out_dir: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InjectMode {
    InlineOneUser,
    TwoUserMessages,
}

#[derive(Debug, Deserialize)]
struct FixturesManifest {
    fixtures: Vec<FixtureSpec>,
}

#[derive(Debug, Deserialize)]
struct FixtureSpec {
    file: String,
    url: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct FfmpegManifest {
    upstream_release_verification: UpstreamReleaseVerification,
    platforms: std::collections::BTreeMap<String, FfmpegPlatformSpec>,
}

#[derive(Debug, Deserialize)]
struct UpstreamReleaseVerification {
    source_url: String,
    source_sig_url: String,
    source_sha256: String,
    signing_key_url: String,
    signing_key_fingerprint: String,
}

#[derive(Debug, Deserialize)]
struct FfmpegPlatformSpec {
    archive_url: String,
    archive_sha256: String,
    archive_type: String,
    archive_root: String,
    ffmpeg_file: String,
    ffprobe_file: String,
    ffmpeg_sha256: String,
    ffprobe_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct WindowsProcessInfo {
    #[serde(rename = "ProcessId")]
    process_id: u32,
    #[serde(rename = "ExecutablePath")]
    executable_path: Option<String>,
    #[serde(rename = "CreationDate")]
    creation_date: Option<String>,
}

#[derive(Debug, Clone)]
struct ContextInputs {
    history_lines: Vec<String>,
    clipboard: Option<String>,
    prev_title: Option<String>,
    prev_process: Option<String>,
}

#[derive(Debug, Clone)]
struct PreprocessConfig {
    silence_trim_enabled: bool,
    silence_threshold_db: f64,
    silence_start_ms: u64,
    silence_end_ms: u64,
}

impl Default for PreprocessConfig {
    fn default() -> Self {
        Self {
            silence_trim_enabled: false,
            silence_threshold_db: -50.0,
            silence_start_ms: 300,
            silence_end_ms: 300,
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("FAIL: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Verify { command } => match command {
            VerifyCommand::Quick => run_verify(VerifyLevel::Quick),
            VerifyCommand::Full => run_verify(VerifyLevel::Full),
        },
        Commands::Fixtures { command } => match command {
            FixturesCommand::Download => {
                ensure_fixtures_ready(&["zh_10s.ogg", "zh_60s.ogg", "zh_5m.ogg"])?;
                println!("OK: fixtures ready");
                Ok(())
            }
        },
        Commands::Toolchain { command } => match command {
            ToolchainCommand::Ffmpeg(args) => install_ffmpeg_toolchain(args.platform),
        },
        Commands::Gate { command } => match command {
            GateCommand::Windows => run_windows_gate(),
            GateCommand::WindowsCompile => run_windows_compile_gate(),
        },
        Commands::Run { command } => match command {
            RunCommand::Latest => run_latest(),
        },
        Commands::LlmPromptLab(args) => run_llm_prompt_lab(args),
    }
}

#[derive(Debug, Clone, Copy)]
enum VerifyLevel {
    Quick,
    Full,
}

impl VerifyLevel {
    fn as_str(self) -> &'static str {
        match self {
            VerifyLevel::Quick => "quick",
            VerifyLevel::Full => "full",
        }
    }
}

fn repo_root() -> Result<PathBuf> {
    if let Ok(raw) = env::var("TYPEVOICE_REPO_ROOT") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("failed to locate repo root from CARGO_MANIFEST_DIR"))
}

fn fixtures_manifest_path() -> Result<PathBuf> {
    if let Ok(raw) = env::var("TYPEVOICE_FIXTURES_MANIFEST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(repo_root()?.join("scripts").join("fixtures_manifest.json"))
}

fn fixtures_dir() -> Result<PathBuf> {
    if let Ok(raw) = env::var("TYPEVOICE_FIXTURES_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(repo_root()?.join("fixtures"))
}

fn load_fixtures_manifest() -> Result<FixturesManifest> {
    let path = fixtures_manifest_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("fixtures manifest missing: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("cannot parse fixtures manifest: {}", path.display()))
}

fn ensure_fixtures_ready(required_files: &[&str]) -> Result<()> {
    let manifest = load_fixtures_manifest()?;
    let dir = fixtures_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("create fixtures dir: {}", dir.display()))?;
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("create http client")?;

    for name in required_files {
        let spec = manifest
            .fixtures
            .iter()
            .find(|item| item.file == *name)
            .ok_or_else(|| anyhow!("fixture not declared in manifest: {name}"))?;
        if spec.url.trim().is_empty() || spec.sha256.trim().is_empty() {
            bail!("fixture spec incomplete for: {name}");
        }

        let target = dir.join(name);
        if target.exists() {
            let got = sha256_file(&target)?;
            if got.eq_ignore_ascii_case(&spec.sha256) {
                continue;
            }
            println!(
                "WARN: fixture checksum mismatch, re-downloading: {}",
                target.display()
            );
        }

        let tmp = target.with_extension(format!(
            "{}download",
            target
                .extension()
                .and_then(|v| v.to_str())
                .map(|v| format!("{v}."))
                .unwrap_or_default()
        ));
        download_file(&client, &spec.url, &tmp)
            .with_context(|| format!("fixture download failed for {name} from {}", spec.url))?;
        let got = sha256_file(&tmp)?;
        if !got.eq_ignore_ascii_case(&spec.sha256) {
            let _ = fs::remove_file(&tmp);
            bail!(
                "fixture checksum mismatch for {name}\n  expected={}\n  actual={got}",
                spec.sha256
            );
        }
        fs::rename(&tmp, &target)
            .with_context(|| format!("install fixture: {}", target.display()))?;
        println!("INFO: fixture ready: {}", target.display());
    }
    Ok(())
}

fn download_file(client: &Client, url: &str, target: &Path) -> Result<()> {
    let mut response = client.get(url).send()?.error_for_status()?;
    let mut out = File::create(target).with_context(|| format!("create {}", target.display()))?;
    io::copy(&mut response, &mut out).with_context(|| format!("write {}", target.display()))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0_u8; 1024 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn ensure_dirs() -> Result<()> {
    let root = repo_root()?;
    fs::create_dir_all(root.join("metrics")).context("create metrics dir")?;
    fs::create_dir_all(root.join("tmp")).context("create tmp dir")?;
    Ok(())
}

fn append_jsonl(path: &Path, obj: &Value) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(f, "{}", serde_json::to_string(obj)?)
        .with_context(|| format!("append {}", path.display()))?;
    Ok(())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn run_verify(level: VerifyLevel) -> Result<()> {
    let root = repo_root()?;
    let started = Instant::now();
    ensure_dirs()?;
    match level {
        VerifyLevel::Quick => ensure_fixtures_ready(&["zh_5m.ogg"])?,
        VerifyLevel::Full => ensure_fixtures_ready(&["zh_10s.ogg", "zh_60s.ogg", "zh_5m.ogg"])?,
    }

    let tauri_dir = root.join("apps").join("desktop").join("src-tauri");
    let metrics_path = root.join("metrics").join("verify.jsonl");

    if let Err(e) = run_native(&tauri_dir, "cargo", &["check", "--locked"]) {
        let record = json!({
            "ts_ms": now_ms(),
            "level": level.as_str(),
            "status": "FAIL",
            "fail_reasons": ["cargo_check_failed"],
        });
        append_jsonl(&metrics_path, &record)?;
        return Err(e.context("cargo check failed"));
    }

    if let Err(e) = run_debuggability_tests(&tauri_dir) {
        let record = json!({
            "ts_ms": now_ms(),
            "level": level.as_str(),
            "status": "FAIL",
            "fail_reasons": ["debuggability_contract_tests_failed"],
        });
        append_jsonl(&metrics_path, &record)?;
        return Err(e.context("debuggability contract tests failed"));
    }

    match level {
        VerifyLevel::Quick => run_native(
            &tauri_dir,
            "cargo",
            &[
                "test",
                "--locked",
                "ffmpeg_preprocess_args_keep_asr_input_format",
            ],
        )?,
        VerifyLevel::Full => run_native(&tauri_dir, "cargo", &["test", "--locked"])?,
    }

    let mut fail_reasons = Vec::<String>::new();
    let mut preprocess = serde_json::Map::new();
    if matches!(level, VerifyLevel::Full) {
        let tmp_dir = root.join("tmp").join("preprocessed");
        fs::create_dir_all(&tmp_dir).context("create preprocessed tmp dir")?;
        for (name, metric) in [
            ("zh_10s.ogg", "10s_ms"),
            ("zh_60s.ogg", "60s_ms"),
            ("zh_5m.ogg", "5m_ms"),
        ] {
            let input = root.join("fixtures").join(name);
            let output = tmp_dir.join(name.replace(".ogg", ".wav"));
            match ffmpeg_preprocess_to_wav(&input, &output) {
                Ok(ms) => {
                    preprocess.insert(metric.to_string(), json!(ms));
                }
                Err(e) => fail_reasons.push(format!("preprocess_failed:{e:#}")),
            }
        }
    }

    let cancel_output = match level {
        VerifyLevel::Quick => root.join("tmp").join("quick_cancel.wav"),
        VerifyLevel::Full => root.join("tmp").join("preprocessed").join("cancel.wav"),
    };
    let cancel_ffmpeg_ms = cancel_ffmpeg_preprocess(
        &root.join("fixtures").join("zh_5m.ogg"),
        &cancel_output,
        100,
    )?;
    if cancel_ffmpeg_ms > 300 {
        fail_reasons.push(format!("cancel_ffmpeg_slow:{cancel_ffmpeg_ms}ms"));
    }

    let status = if fail_reasons.is_empty() {
        "PASS"
    } else {
        "FAIL"
    };
    let total_ms = started.elapsed().as_millis() as i64;
    println!("{status}: cancel_ffmpeg_ms={cancel_ffmpeg_ms} total_ms={total_ms}");

    let mut record = json!({
        "ts_ms": now_ms(),
        "level": level.as_str(),
        "status": status,
        "cancel_ffmpeg_ms": cancel_ffmpeg_ms,
        "fail_reasons": fail_reasons,
    });
    if matches!(level, VerifyLevel::Full) {
        record["preprocess"] = Value::Object(preprocess);
        record["total_ms"] = json!(total_ms);
    }
    append_jsonl(&metrics_path, &record)?;

    if status == "PASS" {
        Ok(())
    } else {
        bail!("verify {} failed", level.as_str())
    }
}

fn run_debuggability_tests(tauri_dir: &Path) -> Result<()> {
    run_native(
        tauri_dir,
        "cargo",
        &[
            "test",
            "--locked",
            "concurrent_emit_keeps_jsonl_lines_parseable",
        ],
    )?;
    run_native(
        tauri_dir,
        "cargo",
        &[
            "test",
            "--locked",
            "concurrent_metrics_emit_keeps_jsonl_lines_parseable",
        ],
    )?;
    Ok(())
}

fn run_native(cwd: &Path, program: &str, args: &[&str]) -> Result<()> {
    run_command(cwd, program, args, &[])
}

fn run_command(cwd: &Path, program: &str, args: &[&str], envs: &[(&str, String)]) -> Result<()> {
    let resolved = resolve_program(program).unwrap_or_else(|| program.to_string());
    let mut command = Command::new(&resolved);
    command.args(args).current_dir(cwd);
    for (key, value) in envs {
        command.env(key, value);
    }
    let status = command
        .status()
        .with_context(|| format!("start command failed: {program} {}", args.join(" ")))?;
    if !status.success() {
        bail!(
            "command failed: {} {} (exit={status})",
            program,
            args.join(" ")
        );
    }
    Ok(())
}

fn run_output(cwd: &Path, program: &str, args: &[&str]) -> Result<Output> {
    let resolved = resolve_program(program).unwrap_or_else(|| program.to_string());
    Command::new(&resolved)
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("start command failed: {program} {}", args.join(" ")))
}

fn ensure_command(name: &str) -> Result<()> {
    if resolve_program(name).is_some() {
        Ok(())
    } else {
        bail!("missing command: {name}")
    }
}

fn resolve_program(program: &str) -> Option<String> {
    let direct = PathBuf::from(program);
    if direct.is_file() {
        return Some(program.to_string());
    }

    if cfg!(windows) {
        if let Ok(output) = Command::new("where.exe").arg(program).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(path) = stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .find(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower.ends_with(".exe")
                            || lower.ends_with(".cmd")
                            || lower.ends_with(".bat")
                    })
                {
                    return Some(path.to_string());
                }
            }
        }
    } else if let Ok(output) = Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(path) = stdout.lines().map(str::trim).find(|line| !line.is_empty()) {
                return Some(path.to_string());
            }
        }
    }

    let names = program_candidate_names(program);
    for entry in path_entries() {
        for candidate in &names {
            let path = entry.join(candidate);
            if path.is_file() {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

fn program_candidate_names(program: &str) -> Vec<String> {
    let path = Path::new(program);
    if path.extension().is_some() || program.contains('/') || program.contains('\\') {
        return vec![program.to_string()];
    }
    if cfg!(windows) {
        vec![
            format!("{program}.exe"),
            format!("{program}.cmd"),
            format!("{program}.bat"),
            program.to_string(),
        ]
    } else {
        vec![program.to_string()]
    }
}

fn path_entries() -> Vec<PathBuf> {
    let raw = env::var("PATH").unwrap_or_default();
    let separator = if raw.contains(';') { ';' } else { ':' };
    raw.split(separator)
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(path_entry_to_host_path(trimmed))
            }
        })
        .collect()
}

fn path_entry_to_host_path(entry: &str) -> PathBuf {
    if cfg!(windows) {
        if let Some(rest) = entry.strip_prefix("/mnt/") {
            let mut chars = rest.chars();
            if let Some(drive) = chars.next() {
                let tail = chars.as_str().trim_start_matches('/').replace('/', "\\");
                return if tail.is_empty() {
                    PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase()))
                } else {
                    PathBuf::from(format!("{}:\\{}", drive.to_ascii_uppercase(), tail))
                };
            }
        }
    }
    PathBuf::from(entry)
}

fn ffmpeg_manifest_path() -> Result<PathBuf> {
    Ok(repo_root()?
        .join("apps")
        .join("desktop")
        .join("src-tauri")
        .join("toolchain")
        .join("ffmpeg_manifest.json"))
}

fn load_ffmpeg_manifest() -> Result<FfmpegManifest> {
    let path = ffmpeg_manifest_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read ffmpeg manifest: {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse ffmpeg manifest: {}", path.display()))
}

fn install_ffmpeg_toolchain(selection: ToolchainPlatform) -> Result<()> {
    let root = repo_root()?;
    let manifest = load_ffmpeg_manifest()?;
    let targets = resolve_toolchain_targets(selection)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("create http client")?;
    let work_root = root
        .join("tmp")
        .join("xtask")
        .join(format!("ffmpeg_{}", now_ms()));
    fs::create_dir_all(&work_root)
        .with_context(|| format!("create work dir: {}", work_root.display()))?;

    let result = (|| {
        verify_ffmpeg_upstream_release(&client, &manifest, &work_root)?;
        for target in targets {
            install_ffmpeg_platform(&client, &manifest, &work_root, &root, &target)?;
        }
        println!("DONE");
        Ok(())
    })();
    let _ = fs::remove_dir_all(&work_root);
    result
}

fn resolve_toolchain_targets(selection: ToolchainPlatform) -> Result<Vec<String>> {
    match selection {
        ToolchainPlatform::All => Ok(vec![
            "windows-x86_64".to_string(),
            "linux-x86_64".to_string(),
        ]),
        ToolchainPlatform::WindowsX86_64 => Ok(vec!["windows-x86_64".to_string()]),
        ToolchainPlatform::LinuxX86_64 => Ok(vec!["linux-x86_64".to_string()]),
        ToolchainPlatform::Auto => {
            if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
                return Ok(vec!["windows-x86_64".to_string()]);
            }
            if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
                return Ok(vec!["linux-x86_64".to_string()]);
            }
            bail!("unsupported host platform for auto mode")
        }
    }
}

fn verify_ffmpeg_upstream_release(
    client: &Client,
    manifest: &FfmpegManifest,
    work_root: &Path,
) -> Result<()> {
    let upstream = &manifest.upstream_release_verification;
    require_non_empty(&upstream.source_url, "source_url")?;
    require_non_empty(&upstream.source_sig_url, "source_sig_url")?;
    require_non_empty(&upstream.source_sha256, "source_sha256")?;
    require_non_empty(&upstream.signing_key_url, "signing_key_url")?;
    require_non_empty(&upstream.signing_key_fingerprint, "signing_key_fingerprint")?;

    println!("INFO: verify FFmpeg upstream release signature");
    let verify_dir = work_root.join("upstream_verify");
    let gnupg_home = verify_dir.join("gnupg");
    fs::create_dir_all(&gnupg_home)
        .with_context(|| format!("create gpg home: {}", gnupg_home.display()))?;
    let key_path = verify_dir.join("ffmpeg-devel.asc");
    let source_path = verify_dir.join("ffmpeg-release.tar.xz");
    let source_sig_path = verify_dir.join("ffmpeg-release.tar.xz.asc");

    download_file(client, &upstream.signing_key_url, &key_path)
        .context("download FFmpeg signing key")?;
    download_file(client, &upstream.source_url, &source_path)
        .context("download FFmpeg source archive")?;
    download_file(client, &upstream.source_sig_url, &source_sig_path)
        .context("download FFmpeg source signature")?;

    let source_sha = sha256_file(&source_path)?;
    if !source_sha.eq_ignore_ascii_case(&upstream.source_sha256) {
        bail!(
            "ffmpeg upstream source sha256 mismatch expected={} actual={source_sha}",
            upstream.source_sha256
        );
    }

    let gpg = resolve_gpg_command();
    run_command(
        &verify_dir,
        &gpg,
        &[
            "--homedir",
            &path_to_string(&gnupg_home)?,
            "--batch",
            "--import",
            &path_to_string(&key_path)?,
        ],
        &[],
    )
    .context("import ffmpeg signing key")?;

    let output = run_output(
        &verify_dir,
        &gpg,
        &[
            "--homedir",
            &path_to_string(&gnupg_home)?,
            "--status-fd=1",
            "--batch",
            "--verify",
            &path_to_string(&source_sig_path)?,
            &path_to_string(&source_path)?,
        ],
    )
    .context("run gpg signature verification")?;
    if !output.status.success() {
        bail!("ffmpeg upstream signature verify command failed");
    }
    let status = String::from_utf8_lossy(&output.stdout);
    if !status
        .lines()
        .any(|line| line.starts_with("[GNUPG:] GOODSIG "))
    {
        bail!("ffmpeg upstream signature verification failed (GOODSIG missing)");
    }
    let actual_fpr = status
        .lines()
        .find_map(|line| {
            line.strip_prefix("[GNUPG:] VALIDSIG ")
                .and_then(|rest| rest.split_whitespace().next())
        })
        .ok_or_else(|| {
            anyhow!("ffmpeg upstream signature verification failed (VALIDSIG missing)")
        })?;
    if !actual_fpr.eq_ignore_ascii_case(&upstream.signing_key_fingerprint) {
        bail!(
            "ffmpeg signing fingerprint mismatch expected={} actual={actual_fpr}",
            upstream.signing_key_fingerprint
        );
    }
    println!("INFO: PASS: ffmpeg upstream signature verified ({actual_fpr})");
    Ok(())
}

fn require_non_empty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("ffmpeg manifest field is empty: {field}");
    }
    Ok(())
}

fn resolve_gpg_command() -> String {
    if cfg!(windows) {
        let common = PathBuf::from(r"D:\Program Files\GnuPG\bin\gpg.exe");
        if common.is_file() {
            return common.display().to_string();
        }
        "gpg.exe".to_string()
    } else {
        "gpg".to_string()
    }
}

fn install_ffmpeg_platform(
    client: &Client,
    manifest: &FfmpegManifest,
    work_root: &Path,
    repo_root: &Path,
    target: &str,
) -> Result<()> {
    println!("INFO: prepare ffmpeg toolchain for {target}");
    let spec = manifest
        .platforms
        .get(target)
        .ok_or_else(|| anyhow!("platform not found in manifest: {target}"))?;

    let work_dir = work_root.join(target);
    let unpack_dir = work_dir.join("unpack");
    fs::create_dir_all(&unpack_dir)
        .with_context(|| format!("create unpack dir: {}", unpack_dir.display()))?;
    let archive_path = work_dir.join(match spec.archive_type.as_str() {
        "zip" => "archive.zip",
        "tar.xz" => "archive.tar.xz",
        other => bail!("unsupported archive_type for {target}: {other}"),
    });

    download_file(client, &spec.archive_url, &archive_path)
        .with_context(|| format!("download failed for {target} url={}", spec.archive_url))?;
    let archive_sha = sha256_file(&archive_path)?;
    if !archive_sha.eq_ignore_ascii_case(&spec.archive_sha256) {
        bail!(
            "archive sha256 mismatch for {target} expected={} actual={archive_sha}",
            spec.archive_sha256
        );
    }

    extract_archive(&archive_path, &unpack_dir, &spec.archive_type)?;

    let src_ffmpeg = source_binary_path(&unpack_dir, spec, &spec.ffmpeg_file);
    let src_ffprobe = source_binary_path(&unpack_dir, spec, &spec.ffprobe_file);
    if !src_ffmpeg.is_file() {
        bail!("missing extracted ffmpeg: {}", src_ffmpeg.display());
    }
    if !src_ffprobe.is_file() {
        bail!("missing extracted ffprobe: {}", src_ffprobe.display());
    }

    let dest_dir = repo_root
        .join("apps")
        .join("desktop")
        .join("src-tauri")
        .join("toolchain")
        .join("bin")
        .join(target);
    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("create destination dir: {}", dest_dir.display()))?;
    let dest_ffmpeg = dest_dir.join(&spec.ffmpeg_file);
    let dest_ffprobe = dest_dir.join(&spec.ffprobe_file);
    fs::copy(&src_ffmpeg, &dest_ffmpeg).with_context(|| {
        format!(
            "copy ffmpeg {} -> {}",
            src_ffmpeg.display(),
            dest_ffmpeg.display()
        )
    })?;
    fs::copy(&src_ffprobe, &dest_ffprobe).with_context(|| {
        format!(
            "copy ffprobe {} -> {}",
            src_ffprobe.display(),
            dest_ffprobe.display()
        )
    })?;
    make_executable_if_needed(target, &dest_ffmpeg)?;
    make_executable_if_needed(target, &dest_ffprobe)?;

    let ffmpeg_sha = sha256_file(&dest_ffmpeg)?;
    if !ffmpeg_sha.eq_ignore_ascii_case(&spec.ffmpeg_sha256) {
        bail!(
            "ffmpeg sha256 mismatch for {target} expected={} actual={ffmpeg_sha}",
            spec.ffmpeg_sha256
        );
    }
    let ffprobe_sha = sha256_file(&dest_ffprobe)?;
    if !ffprobe_sha.eq_ignore_ascii_case(&spec.ffprobe_sha256) {
        bail!(
            "ffprobe sha256 mismatch for {target} expected={} actual={ffprobe_sha}",
            spec.ffprobe_sha256
        );
    }

    println!("INFO: PASS: {target} -> {}", dest_dir.display());
    Ok(())
}

fn source_binary_path(unpack_dir: &Path, spec: &FfmpegPlatformSpec, file_name: &str) -> PathBuf {
    let base = unpack_dir.join(&spec.archive_root);
    if spec.archive_type == "zip" {
        base.join("bin").join(file_name)
    } else {
        base.join(file_name)
    }
}

fn extract_archive(archive_path: &Path, unpack_dir: &Path, archive_type: &str) -> Result<()> {
    match archive_type {
        "zip" if cfg!(windows) => run_command(
            unpack_dir,
            "tar.exe",
            &[
                "-xf",
                &path_to_string(archive_path)?,
                "-C",
                &path_to_string(unpack_dir)?,
            ],
            &[],
        ),
        "zip" => run_command(
            unpack_dir,
            "unzip",
            &[
                "-q",
                &path_to_string(archive_path)?,
                "-d",
                &path_to_string(unpack_dir)?,
            ],
            &[],
        ),
        "tar.xz" => run_command(
            unpack_dir,
            if cfg!(windows) { "tar.exe" } else { "tar" },
            &[
                "-xf",
                &path_to_string(archive_path)?,
                "-C",
                &path_to_string(unpack_dir)?,
            ],
            &[],
        ),
        other => bail!("unsupported archive_type: {other}"),
    }
}

#[cfg(unix)]
fn make_executable_if_needed(target: &str, path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if target == "linux-x86_64" {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_executable_if_needed(_target: &str, _path: &Path) -> Result<()> {
    Ok(())
}

fn run_windows_compile_gate() -> Result<()> {
    ensure_windows_runtime("gate windows-compile")?;
    let root = repo_root()?;
    if !root
        .join("apps")
        .join("desktop")
        .join("src-tauri")
        .join("Cargo.toml")
        .is_file()
    {
        bail!("run this command from the repo root");
    }
    ensure_command("cargo")?;
    println!("INFO: running: cargo check (Windows compile gate)");
    let tauri_dir = root.join("apps").join("desktop").join("src-tauri");
    let mut envs = vec![("CARGO_INCREMENTAL", "0".to_string())];
    let cwd = path_to_string(&tauri_dir)?;
    if env::var("CARGO_TARGET_DIR")
        .unwrap_or_default()
        .trim()
        .is_empty()
        && cwd.starts_with(r"\\")
    {
        envs.push((
            "CARGO_TARGET_DIR",
            env::temp_dir()
                .join(format!("typevoice-target-{}", now_ms()))
                .display()
                .to_string(),
        ));
    }
    run_command(&tauri_dir, "cargo", &["check", "--locked"], &envs)?;
    println!("PASS");
    Ok(())
}

fn run_windows_gate() -> Result<()> {
    ensure_windows_runtime("gate windows")?;
    let root = repo_root()?;
    println!("INFO: repo_root={}", root.display());
    for command in ["node", "npm", "cargo", "gpg"] {
        ensure_command(command)?;
    }
    run_windows_compile_gate()?;
    try_enable_sccache(&root)?;
    install_ffmpeg_toolchain(ToolchainPlatform::All)?;

    let toolchain_dir = root
        .join("apps")
        .join("desktop")
        .join("src-tauri")
        .join("toolchain")
        .join("bin")
        .join("windows-x86_64");
    env::set_var(
        "TYPEVOICE_TOOLCHAIN_DIR",
        toolchain_dir.display().to_string(),
    );
    env::set_var(
        "TYPEVOICE_FFMPEG",
        toolchain_dir.join("ffmpeg.exe").display().to_string(),
    );
    env::set_var(
        "TYPEVOICE_FFPROBE",
        toolchain_dir.join("ffprobe.exe").display().to_string(),
    );

    let desktop_dir = root.join("apps").join("desktop");
    kill_stale_windows_processes(&root)?;
    println!("INFO: installing desktop npm deps");
    run_command(&desktop_dir, "npm", &["ci"], &[])?;
    println!("INFO: running desktop build");
    run_command(&desktop_dir, "npm", &["run", "build"], &[])?;
    println!("INFO: running rust tests");
    run_command(
        &root.join("apps").join("desktop").join("src-tauri"),
        "cargo",
        &["test", "--locked"],
        &[],
    )?;
    start_tauri_dev_and_wait(&root, &desktop_dir, "gate-windows")
}

fn try_enable_sccache(repo_root: &Path) -> Result<()> {
    if ensure_command("sccache").is_ok() {
        let cache_dir = repo_root.join(".cache").join("sccache");
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create sccache dir: {}", cache_dir.display()))?;
        env::set_var("RUSTC_WRAPPER", "sccache");
        env::set_var("SCCACHE_DIR", cache_dir.display().to_string());
        println!(
            "INFO: sccache enabled: RUSTC_WRAPPER=sccache SCCACHE_DIR={}",
            cache_dir.display()
        );
    } else {
        println!("INFO: sccache not found (optional). Install once with: cargo install sccache");
    }
    Ok(())
}

fn run_latest() -> Result<()> {
    ensure_windows_runtime("run latest")?;
    let root = repo_root()?;
    let desktop_dir = root.join("apps").join("desktop");
    kill_stale_windows_processes(&root)?;
    println!("INFO: installing desktop npm deps");
    run_command(&desktop_dir, "npm", &["ci"], &[])?;
    println!("INFO: building frontend bundle (npm run build)");
    run_command(&desktop_dir, "npm", &["run", "build"], &[])?;
    start_tauri_dev_and_wait(&root, &desktop_dir, "run-latest")
}

fn start_tauri_dev_and_wait(root: &Path, desktop_dir: &Path, label: &str) -> Result<()> {
    let log_dir = root.join("tmp").join("typevoice-logs");
    let log_file = log_dir.join("tauri-latest-run.txt");
    let frontend_binary = root
        .join("target")
        .join("debug")
        .join("typevoice-desktop.exe");

    fs::create_dir_all(&log_dir)
        .with_context(|| format!("create log dir: {}", log_dir.display()))?;
    fs::write(
        &log_file,
        format!(
            "\r\n=== cargo xtask {label} {} ===\r\n",
            Utc::now().to_rfc3339()
        ),
    )
    .with_context(|| format!("write log header: {}", log_file.display()))?;

    println!(
        "INFO: starting tauri dev with logs redirected to {}",
        log_file.display()
    );
    let dev_pid = start_tauri_dev_background(&desktop_dir, &log_file)?;

    println!(
        "INFO: waiting for runtime process to appear: {}",
        frontend_binary.display()
    );
    match wait_for_windows_process(&frontend_binary, Duration::from_secs(180))? {
        Some(runtime) => {
            println!("INFO: runtime started successfully");
            println!("PID: {}", runtime.process_id);
            println!(
                "Path: {}",
                runtime
                    .executable_path
                    .unwrap_or_else(|| "<unknown>".to_string())
            );
            println!(
                "StartTime: {}",
                runtime
                    .creation_date
                    .unwrap_or_else(|| "<unknown>".to_string())
            );
            println!("Log: {}", log_file.display());
            println!("Last 30 log lines:");
            for line in tail_lines(&log_file, 30)? {
                println!("{line}");
            }
            Ok(())
        }
        None => {
            println!("dev command pid={dev_pid}");
            bail!(
                "typevoice-desktop.exe did not start. Check log: {}",
                log_file.display()
            )
        }
    }
}

fn start_tauri_dev_background(desktop_dir: &Path, log_file: &Path) -> Result<u32> {
    let tauri_js = desktop_dir
        .join("node_modules")
        .join("@tauri-apps")
        .join("cli")
        .join("tauri.js");
    if !tauri_js.is_file() {
        bail!("missing local tauri cli: {}", tauri_js.display());
    }
    let node = resolve_program("node").ok_or_else(|| anyhow!("missing command: node"))?;
    let launcher = log_file
        .parent()
        .ok_or_else(|| anyhow!("log file has no parent: {}", log_file.display()))?
        .join("run-latest.cmd");
    let before_dev = log_file
        .parent()
        .ok_or_else(|| anyhow!("log file has no parent: {}", log_file.display()))?
        .join("before-dev.cmd");
    let config_path = log_file
        .parent()
        .ok_or_else(|| anyhow!("log file has no parent: {}", log_file.display()))?
        .join("tauri-dev-config.json");
    let vite_js = desktop_dir
        .join("node_modules")
        .join("vite")
        .join("bin")
        .join("vite.js");
    if !vite_js.is_file() {
        bail!("missing local vite cli: {}", vite_js.display());
    }
    let before_dev_body = format!(
        "@echo off\r\nset \"PATH={};%PATH%\"\r\ncd /d \"{}\"\r\n\"{}\" \"{}\"\r\n",
        required_windows_tool_path()?,
        path_to_string(desktop_dir)?,
        node,
        path_to_string(&vite_js)?,
    );
    fs::write(&before_dev, before_dev_body)
        .with_context(|| format!("write beforeDevCommand launcher: {}", before_dev.display()))?;
    write_json_pretty(
        &config_path,
        &json!({
            "build": {
                "beforeDevCommand": path_to_string(&before_dev)?,
            }
        }),
    )?;
    let launcher_body = format!(
        "@echo off\r\nset \"PATH={};%PATH%\"\r\ncd /d \"{}\"\r\nset RUST_BACKTRACE=1\r\nset RUST_LOG=debug\r\n\"{}\" \"{}\" dev -c \"{}\" >> \"{}\" 2>&1\r\n",
        required_windows_tool_path()?,
        path_to_string(desktop_dir)?,
        node,
        path_to_string(&tauri_js)?,
        path_to_string(&config_path)?,
        path_to_string(log_file)?,
    );
    fs::write(&launcher, launcher_body)
        .with_context(|| format!("write launcher: {}", launcher.display()))?;
    let launcher = ps_double_quote(&path_to_string(&launcher)?);
    let script = format!(
        r#"$ErrorActionPreference='Stop';
$p = Start-Process -FilePath 'cmd.exe' -ArgumentList @('/d','/c','"{launcher}"') -WindowStyle Hidden -PassThru;
Write-Host $p.Id;"#
    );
    let output = powershell_output(&script)?;
    if !output.status.success() {
        bail!(
            "failed to start tauri dev: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let pid = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| anyhow!("Start-Process did not return a pid"))?
        .parse::<u32>()
        .context("parse Start-Process pid")?;
    Ok(pid)
}

fn required_windows_tool_path() -> Result<String> {
    let mut dirs = Vec::<String>::new();
    for program in ["node", "npm", "cargo"] {
        let path = resolve_program(program).ok_or_else(|| anyhow!("missing command: {program}"))?;
        let parent = Path::new(&path)
            .parent()
            .ok_or_else(|| anyhow!("cannot determine command directory: {path}"))?
            .display()
            .to_string();
        if !dirs.iter().any(|dir| dir.eq_ignore_ascii_case(&parent)) {
            dirs.push(parent);
        }
    }
    Ok(dirs.join(";"))
}

fn ensure_windows_runtime(command: &str) -> Result<()> {
    if cfg!(windows) {
        Ok(())
    } else {
        bail!("{command} requires the Windows Rust runtime")
    }
}

fn kill_stale_windows_processes(repo_root: &Path) -> Result<()> {
    println!("INFO: stopping stale TypeVoice Windows processes");
    let root = ps_single_quote(&path_to_string(repo_root)?.to_lowercase());
    let script = format!(
        r#"$ErrorActionPreference='Stop';
$root='{root}';
Get-CimInstance Win32_Process -Filter "Name='typevoice-desktop.exe'" | ForEach-Object {{
  Write-Host ('killing typevoice-desktop.exe: pid=' + $_.ProcessId);
  Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}};
Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object {{
  $_.CommandLine -and $_.CommandLine.ToLowerInvariant().Contains($root)
}} | ForEach-Object {{
  Write-Host ('killing node.exe from workspace: pid=' + $_.ProcessId);
  Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}};
Start-Sleep -Seconds 1;
if (Get-Process -Name typevoice-desktop -ErrorAction SilentlyContinue) {{
  Write-Error 'failed to stop stale typevoice-desktop.exe';
  exit 1;
}}"#
    );
    let output = powershell_output(&script)?;
    if !output.status.success() {
        bail!(
            "failed to stop stale TypeVoice processes: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

fn wait_for_windows_process(
    exe_path: &Path,
    timeout: Duration,
) -> Result<Option<WindowsProcessInfo>> {
    let target = ps_single_quote(&path_to_string(exe_path)?.to_lowercase());
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let script = format!(
            r#"$target='{target}';
$proc = Get-CimInstance Win32_Process -Filter "Name='typevoice-desktop.exe'" | Where-Object {{
  ($_.ExecutablePath -and $_.ExecutablePath.ToLowerInvariant() -eq $target) -or
  ($_.CommandLine -and $_.CommandLine.ToLowerInvariant().Contains($target))
}} | Select-Object -First 1 ProcessId,ExecutablePath,CreationDate;
if ($proc) {{ $proc | ConvertTo-Json -Compress }}"#
        );
        let output = powershell_output(&script)?;
        if !output.status.success() {
            bail!(
                "process query failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !raw.is_empty() {
            let info = serde_json::from_str(&raw)
                .with_context(|| format!("parse process query output: {raw}"))?;
            return Ok(Some(info));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(None)
}

fn powershell_output(script: &str) -> Result<Output> {
    let powershell =
        resolve_program("powershell.exe").unwrap_or_else(|| "powershell.exe".to_string());
    Command::new(powershell)
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .context("start powershell.exe")
}

fn ps_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn ps_double_quote(value: &str) -> String {
    value.replace('`', "``").replace('"', "`\"")
}

fn tail_lines(path: &Path, n: usize) -> Result<Vec<String>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let lines = raw
        .lines()
        .rev()
        .take(n)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    Ok(lines.into_iter().rev().collect())
}

fn resolve_tool_binary(env_key: &str, file_name: &str) -> Result<PathBuf> {
    if let Ok(raw) = env::var(env_key) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_file() {
                return Ok(path);
            }
            bail!("{env_key} points to missing file: {}", path.display());
        }
    }
    let dir = if let Ok(raw) = env::var("TYPEVOICE_TOOLCHAIN_DIR") {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            default_toolchain_dir()?
        } else {
            PathBuf::from(trimmed)
        }
    } else {
        default_toolchain_dir()?
    };
    let path = dir.join(file_name);
    if path.is_file() {
        Ok(path)
    } else {
        bail!(
            "missing tool binary {} (set {env_key} or TYPEVOICE_TOOLCHAIN_DIR)",
            path.display()
        )
    }
}

fn default_toolchain_dir() -> Result<PathBuf> {
    let platform = if cfg!(windows) {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    };
    Ok(repo_root()?
        .join("apps")
        .join("desktop")
        .join("src-tauri")
        .join("toolchain")
        .join("bin")
        .join(platform))
}

fn ffmpeg_binary() -> Result<PathBuf> {
    resolve_tool_binary(
        "TYPEVOICE_FFMPEG",
        if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        },
    )
}

fn build_ffmpeg_preprocess_args(
    input_path: &Path,
    output_path: &Path,
    cfg: &PreprocessConfig,
) -> Result<Vec<String>> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-i".to_string(),
        path_to_string(input_path)?,
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        "16000".to_string(),
        "-c:a".to_string(),
        "pcm_s16le".to_string(),
    ];
    if cfg.silence_trim_enabled {
        let start = cfg.silence_start_ms as f64 / 1000.0;
        let end = cfg.silence_end_ms as f64 / 1000.0;
        args.extend([
            "-af".to_string(),
            format!(
                "silenceremove=start_periods=1:start_duration={start:.3}:start_threshold={:.2}dB:stop_periods=-1:stop_duration={end:.3}:stop_threshold={:.2}dB",
                cfg.silence_threshold_db, cfg.silence_threshold_db
            ),
        ]);
    }
    args.extend(["-vn".to_string(), path_to_string(output_path)?]);
    Ok(args)
}

fn path_to_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("non-utf8 path: {}", path.display()))
}

fn ffmpeg_preprocess_to_wav(input: &Path, output: &Path) -> Result<u128> {
    let ffmpeg = ffmpeg_binary()?;
    let args = build_ffmpeg_preprocess_args(input, output, &PreprocessConfig::default())?;
    let started = Instant::now();
    let status = Command::new(&ffmpeg)
        .args(args)
        .status()
        .with_context(|| format!("start ffmpeg: {}", ffmpeg.display()))?;
    if !status.success() {
        bail!("ffmpeg preprocess failed: exit={status}");
    }
    Ok(started.elapsed().as_millis())
}

fn cancel_ffmpeg_preprocess(input: &Path, output: &Path, delay_ms: u64) -> Result<u128> {
    let ffmpeg = ffmpeg_binary()?;
    let args = build_ffmpeg_preprocess_args(input, output, &PreprocessConfig::default())?;
    let mut child = Command::new(&ffmpeg)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("start ffmpeg: {}", ffmpeg.display()))?;
    let started = Instant::now();
    std::thread::sleep(Duration::from_millis(delay_ms));
    let _ = child.kill();
    let _ = child.wait();
    Ok(started.elapsed().as_millis())
}

fn run_llm_prompt_lab(args: LlmPromptLabArgs) -> Result<()> {
    let base_url_raw = if args.base_url.trim().is_empty() {
        env::var("TYPEVOICE_LLM_BASE_URL").unwrap_or_default()
    } else {
        args.base_url.clone()
    };
    let base_url = normalize_base_url(&base_url_raw);
    let model = if args.model.trim().is_empty() {
        env::var("TYPEVOICE_LLM_MODEL").unwrap_or_default()
    } else {
        args.model.clone()
    };
    let model = model.trim().to_string();
    if model.is_empty() {
        bail!("--model is required (or TYPEVOICE_LLM_MODEL)");
    }
    let reasoning_effort = if args.reasoning_effort.trim().is_empty() {
        env::var("TYPEVOICE_LLM_REASONING_EFFORT").unwrap_or_default()
    } else {
        args.reasoning_effort.clone()
    };
    let api_key = if args.api_key.trim().is_empty() {
        env::var("TYPEVOICE_LLM_API_KEY").unwrap_or_default()
    } else {
        args.api_key.clone()
    };
    let api_key = api_key.trim().to_string();

    let mut system_prompt = args.system_prompt.trim().to_string();
    if let Some(path) = &args.system_prompt_file {
        if args.edit {
            open_in_editor(path)?;
        }
        system_prompt = read_text(path)?;
    } else if args.edit {
        bail!("--edit requires --system-prompt-file");
    }

    let mut transcript = args.transcript.trim().to_string();
    if let Some(path) = &args.transcript_file {
        transcript = read_text(path)?.trim().to_string();
    }
    if transcript.is_empty() {
        bail!("transcript is empty (provide --transcript or --transcript-file)");
    }

    let history_lines = match &args.history_file {
        Some(path) => read_text(path)?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        None => Vec::new(),
    };
    let clipboard_text = if let Some(path) = &args.clipboard_file {
        read_text(path)?.trim().to_string()
    } else {
        args.clipboard.trim().to_string()
    };
    let ctx = ContextInputs {
        history_lines,
        clipboard: optional_trimmed(clipboard_text),
        prev_title: optional_trimmed(args.prev_title),
        prev_process: optional_trimmed(args.prev_process),
    };

    let messages = build_messages(
        args.inject_mode,
        &system_prompt,
        &transcript,
        &ctx,
        args.max_history_items,
        args.max_chars_per_history,
        args.max_chars_clipboard,
    );
    let mut req_body = json!({
        "model": model,
        "messages": messages,
        "temperature": 0.2,
    });
    let reasoning_effort = reasoning_effort.trim().to_string();
    if !reasoning_effort.is_empty() && !reasoning_effort.eq_ignore_ascii_case("default") {
        req_body["reasoning_effort"] = json!(reasoning_effort);
    }

    let material = json!({
        "inject_mode": inject_mode_name(args.inject_mode),
        "system_prompt": system_prompt,
        "transcript": transcript,
        "ctx": {
            "history": ctx.history_lines.iter().take(args.max_history_items).cloned().collect::<Vec<_>>(),
            "clipboard": ctx.clipboard,
            "prev_title": ctx.prev_title,
            "prev_process": ctx.prev_process,
        },
        "model": req_body["model"],
        "reasoning_effort": if reasoning_effort.is_empty() { Value::Null } else { json!(reasoning_effort) },
    });
    let material_bytes = serde_json::to_vec(&material)?;
    let short_hash = &sha256_bytes(&material_bytes)[..12];

    let out_dir = if args.out_dir.trim().is_empty() {
        repo_root()?
            .join("tmp")
            .join("llm_prompt_lab")
            .join(format!(
                "{}_{}",
                Utc::now().format("%Y%m%d_%H%M%S"),
                short_hash
            ))
    } else {
        PathBuf::from(args.out_dir.trim())
    };
    fs::create_dir_all(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;

    let meta = json!({
        "ts_utc": Utc::now().to_rfc3339(),
        "base_url": base_url,
        "endpoint": format!("{base_url}/chat/completions"),
        "model": req_body["model"],
        "reasoning_effort": if reasoning_effort.is_empty() { Value::Null } else { json!(reasoning_effort) },
        "inject_mode": inject_mode_name(args.inject_mode),
        "system_prompt_sha256": sha256_bytes(system_prompt.as_bytes()),
        "inputs_sha256": sha256_bytes(&material_bytes),
    });
    write_json_pretty(&out_dir.join("meta.json"), &meta)?;
    write_json_pretty(&out_dir.join("request.json"), &req_body)?;

    if args.dry_run {
        println!("=== REQUEST ===");
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"url": format!("{base_url}/chat/completions"), "body": req_body})
            )?
        );
        if args.print_out_dir {
            println!("=== OUT_DIR ===");
            println!("{}", out_dir.display());
        }
        return Ok(());
    }

    let client = Client::builder()
        .timeout(Duration::from_secs_f64(args.timeout_s))
        .build()
        .context("create http client")?;
    let mut request = client
        .post(format!("{base_url}/chat/completions"))
        .json(&req_body);
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().context("send llm request")?;
    let status = response.status().as_u16();
    let raw = response.text().context("read llm response")?;
    fs::write(out_dir.join("response_raw.txt"), &raw)?;
    fs::write(out_dir.join("http_status.txt"), format!("{status}\n"))?;

    if !(200..300).contains(&status) {
        fs::write(out_dir.join("error.txt"), format!("HTTP {status}\n"))?;
        println!("=== REQUEST ===");
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"url": format!("{base_url}/chat/completions"), "body": req_body})
            )?
        );
        println!("=== RESPONSE_RAW ===");
        println!("{raw}");
        println!("=== HTTP_STATUS ===");
        println!("{status}");
        if args.print_out_dir {
            println!("=== OUT_DIR ===");
            println!("{}", out_dir.display());
        }
        bail!("llm request failed with HTTP {status}");
    }

    let resp_obj: Value = serde_json::from_str(&raw).map_err(|e| {
        let _ = fs::write(
            out_dir.join("error.txt"),
            format!("json_parse_failed: {e}\n"),
        );
        anyhow!("json_parse_failed: {e}")
    })?;
    write_json_pretty(&out_dir.join("response.json"), &resp_obj)?;
    let content = resp_obj
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    fs::write(out_dir.join("response.txt"), format!("{content}\n"))?;

    println!("=== REQUEST ===");
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"url": format!("{base_url}/chat/completions"), "body": req_body})
        )?
    );
    println!("=== RESPONSE_RAW ===");
    println!("{raw}");
    println!("=== PARSED_CONTENT ===");
    println!("{content}");
    if args.print_out_dir {
        println!("=== OUT_DIR ===");
        println!("{}", out_dir.display());
    }
    Ok(())
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

fn write_json_pretty(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")
        .with_context(|| format!("write {}", path.display()))
}

fn optional_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn open_in_editor(path: &Path) -> Result<()> {
    let editor = env::var("EDITOR")
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    if editor.is_empty() {
        bail!("--edit requires EDITOR to be set");
    }
    let status = Command::new(editor)
        .arg(path)
        .status()
        .with_context(|| format!("open editor for {}", path.display()))?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

fn normalize_base_url(value: &str) -> String {
    let mut out = value.trim().trim_end_matches('/').to_string();
    if out.is_empty() {
        out = "https://api.openai.com/v1".to_string();
    }
    if let Some(stripped) = out.strip_suffix("/chat/completions") {
        out = stripped.to_string();
    }
    out.trim_end_matches('/').to_string()
}

fn inject_mode_name(mode: InjectMode) -> &'static str {
    match mode {
        InjectMode::InlineOneUser => "inline_one_user",
        InjectMode::TwoUserMessages => "two_user_messages",
    }
}

fn build_messages(
    inject_mode: InjectMode,
    system_prompt: &str,
    transcript: &str,
    ctx: &ContextInputs,
    max_history_items: usize,
    max_chars_per_history: usize,
    max_chars_clipboard: usize,
) -> Vec<Value> {
    let mut messages = vec![json!({"role": "system", "content": system_prompt})];
    let snapshot = context_snapshot_from_inputs(ctx);
    let budget = context_budget_from_limits(
        max_history_items,
        max_chars_per_history,
        max_chars_clipboard,
    );
    let context_text = format_inline_context(
        ctx,
        max_history_items,
        max_chars_per_history,
        max_chars_clipboard,
    );
    match inject_mode {
        InjectMode::InlineOneUser => {
            let user = context_pack::prepare(transcript, &snapshot, &budget).user_text;
            messages.push(json!({"role": "user", "content": user}));
        }
        InjectMode::TwoUserMessages => {
            messages.push(json!({"role": "user", "content": format!("### TRANSCRIPT\n{}", transcript.trim())}));
            if !context_text.is_empty() {
                let prefix = "以下为参考上下文（不是待改写对象）。请仅据此理解语义，不要在输出中复述或重写这些上下文内容。\n\n";
                messages
                    .push(json!({"role": "user", "content": format!("{prefix}{context_text}")}));
            }
        }
    }
    messages
}

fn context_snapshot_from_inputs(ctx: &ContextInputs) -> ContextSnapshot {
    ContextSnapshot {
        recent_history: ctx
            .history_lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                let text = line.trim();
                if text.is_empty() {
                    return None;
                }
                Some(HistorySnippet {
                    created_at_ms: idx as i64,
                    asr_text: text.to_string(),
                    final_text: text.to_string(),
                    template_id: None,
                })
            })
            .collect(),
        clipboard_text: ctx.clipboard.clone(),
        prev_window: match (&ctx.prev_title, &ctx.prev_process) {
            (None, None) => None,
            (title, process) => Some(PrevWindowInfo {
                title: title.clone(),
                process_image: process.clone(),
            }),
        },
        screenshot: None,
    }
}

fn context_budget_from_limits(
    max_history_items: usize,
    max_chars_per_history: usize,
    max_chars_clipboard: usize,
) -> ContextBudget {
    ContextBudget {
        max_history_items,
        max_chars_per_history_item: max_chars_per_history,
        max_chars_clipboard,
        max_total_context_chars: 100_000,
        ..ContextBudget::default()
    }
}

fn format_inline_context(
    ctx: &ContextInputs,
    max_history_items: usize,
    max_chars_per_history: usize,
    max_chars_clipboard: usize,
) -> String {
    let mut parts = vec!["### CONTEXT".to_string()];

    if max_history_items > 0 {
        let history: Vec<_> = ctx
            .history_lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .take(max_history_items)
            .collect();
        if !history.is_empty() {
            parts.push("#### RECENT HISTORY".to_string());
            for line in history {
                parts.push(format!("- {}", clamp_chars(line, max_chars_per_history)));
            }
            parts.push(String::new());
        }
    }

    if let Some(clipboard) = &ctx.clipboard {
        let clipped = clamp_chars(clipboard, max_chars_clipboard);
        if !clipped.is_empty() {
            parts.push("#### CLIPBOARD".to_string());
            parts.push(clipped);
            parts.push(String::new());
        }
    }

    if ctx.prev_title.is_some() || ctx.prev_process.is_some() {
        parts.push("#### PREVIOUS WINDOW".to_string());
        if let Some(title) = &ctx.prev_title {
            parts.push(format!("title={}", clamp_chars(title, 200)));
        }
        if let Some(process) = &ctx.prev_process {
            parts.push(format!("process={}", clamp_chars(process, 260)));
        }
        parts.push(String::new());
    }

    parts.join("\n").trim().to_string()
}

fn clamp_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    value
        .trim()
        .chars()
        .filter(|ch| *ch != '\0')
        .take(max_chars)
        .collect()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_url_accepts_endpoint_or_base() {
        assert_eq!(
            normalize_base_url("http://api.server/v1/chat/completions"),
            "http://api.server/v1"
        );
        assert_eq!(
            normalize_base_url("http://api.server/v1/"),
            "http://api.server/v1"
        );
        assert_eq!(normalize_base_url(""), "https://api.openai.com/v1");
    }

    #[test]
    fn ffmpeg_preprocess_args_keep_asr_input_format() {
        let args = build_ffmpeg_preprocess_args(
            Path::new("in.ogg"),
            Path::new("out.wav"),
            &PreprocessConfig::default(),
        )
        .expect("args");

        assert_eq!(args[args.iter().position(|v| v == "-ac").unwrap() + 1], "1");
        assert_eq!(
            args[args.iter().position(|v| v == "-ar").unwrap() + 1],
            "16000"
        );
        assert_eq!(
            args[args.iter().position(|v| v == "-c:a").unwrap() + 1],
            "pcm_s16le"
        );
        assert_eq!(args.last().map(String::as_str), Some("out.wav"));
    }

    #[test]
    fn build_messages_supports_two_user_messages() {
        let ctx = ContextInputs {
            history_lines: vec!["history".to_string()],
            clipboard: Some("clip".to_string()),
            prev_title: Some("title".to_string()),
            prev_process: Some("proc".to_string()),
        };
        let messages = build_messages(
            InjectMode::TwoUserMessages,
            "sys",
            "text",
            &ctx,
            3,
            600,
            800,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert!(messages[2]["content"]
            .as_str()
            .expect("content")
            .contains("RECENT HISTORY"));
    }
}
