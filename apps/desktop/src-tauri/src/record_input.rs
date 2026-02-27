use std::path::Path;

use crate::audio_devices_windows::{self, AudioEndpointInfo, DefaultCaptureRole};
use crate::settings::{self, Settings};

const STRATEGY_FOLLOW_DEFAULT: &str = "follow_default";
const STRATEGY_FIXED_DEVICE: &str = "fixed_device";
const STRATEGY_AUTO_SELECT: &str = "auto_select";
const ROLE_COMMUNICATIONS: &str = "communications";
const ROLE_CONSOLE: &str = "console";

#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioCaptureDeviceView {
    pub endpoint_id: String,
    pub friendly_name: String,
    pub is_default_communications: bool,
    pub is_default_console: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedRecordInput {
    pub spec: String,
    pub strategy_used: String,
    pub endpoint_id: Option<String>,
    pub friendly_name: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum InputStrategy {
    FollowDefault,
    FixedDevice,
    AutoSelect,
}

impl InputStrategy {
    fn as_str(self) -> &'static str {
        match self {
            InputStrategy::FollowDefault => STRATEGY_FOLLOW_DEFAULT,
            InputStrategy::FixedDevice => STRATEGY_FIXED_DEVICE,
            InputStrategy::AutoSelect => STRATEGY_AUTO_SELECT,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DefaultRole {
    Communications,
    Console,
}

impl DefaultRole {
    fn to_windows_role(self) -> DefaultCaptureRole {
        match self {
            DefaultRole::Communications => DefaultCaptureRole::Communications,
            DefaultRole::Console => DefaultCaptureRole::Console,
        }
    }
}

#[derive(Debug, Clone)]
struct DshowDevice {
    name: String,
    alternative_name: Option<String>,
}

#[derive(Debug)]
struct AutoCandidate {
    spec: String,
    display_name: String,
    score: i32,
    order: usize,
}

fn parse_strategy(settings: &Settings) -> Result<InputStrategy, String> {
    let raw = settings
        .record_input_strategy
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(STRATEGY_FOLLOW_DEFAULT)
        .to_ascii_lowercase();
    match raw.as_str() {
        STRATEGY_FOLLOW_DEFAULT => Ok(InputStrategy::FollowDefault),
        STRATEGY_FIXED_DEVICE => Ok(InputStrategy::FixedDevice),
        STRATEGY_AUTO_SELECT => Ok(InputStrategy::AutoSelect),
        _ => Err(format!(
            "E_RECORD_INPUT_STRATEGY_INVALID: unsupported record_input_strategy={raw}"
        )),
    }
}

fn parse_default_role(settings: &Settings) -> Result<DefaultRole, String> {
    let raw = settings
        .record_follow_default_role
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(ROLE_COMMUNICATIONS)
        .to_ascii_lowercase();
    match raw.as_str() {
        ROLE_COMMUNICATIONS | "ecommunications" => Ok(DefaultRole::Communications),
        ROLE_CONSOLE | "econsole" => Ok(DefaultRole::Console),
        _ => Err(format!(
            "E_RECORD_INPUT_ROLE_INVALID: unsupported record_follow_default_role={raw}"
        )),
    }
}

fn collapse_ws_lower(v: &str) -> String {
    v.split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn match_priority(endpoint_name: &str, candidate_name: &str) -> Option<u8> {
    if endpoint_name.eq_ignore_ascii_case(candidate_name) {
        return Some(0);
    }
    let lhs = collapse_ws_lower(endpoint_name);
    let rhs = collapse_ws_lower(candidate_name);
    if lhs == rhs {
        return Some(1);
    }
    if lhs.contains(&rhs) || rhs.contains(&lhs) {
        return Some(2);
    }
    None
}

fn normalize_record_input_spec(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("audio=") {
        let value = rest.trim();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            let first = bytes[0] as char;
            let last = bytes[value.len() - 1] as char;
            if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
                return format!("audio={}", &value[1..value.len() - 1]);
            }
        }
    }
    trimmed.to_string()
}

fn first_quoted_token(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let tail = &line[start + 1..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

fn parse_dshow_audio_devices(stderr: &str) -> Vec<DshowDevice> {
    let mut devices: Vec<DshowDevice> = Vec::new();
    let mut pending_idx: Option<usize> = None;
    for line in stderr.lines() {
        let text = line.trim();
        if text.contains("Alternative name") {
            if let (Some(idx), Some(alt)) = (pending_idx, first_quoted_token(text)) {
                if let Some(slot) = devices.get_mut(idx) {
                    slot.alternative_name = Some(alt);
                }
            }
            continue;
        }
        if !text.contains("(audio)") {
            continue;
        }
        if let Some(name) = first_quoted_token(text) {
            devices.push(DshowDevice {
                name,
                alternative_name: None,
            });
            pending_idx = Some(devices.len() - 1);
        }
    }
    devices
}

fn list_dshow_audio_devices(ffmpeg: &Path) -> Result<Vec<DshowDevice>, String> {
    let output = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            format!("E_RECORD_INPUT_DISCOVERY_FAILED: enumerate dshow device failed: {e}")
        })?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let devices = parse_dshow_audio_devices(&stderr);
    if devices.is_empty() {
        return Err("E_RECORD_INPUT_DISCOVERY_FAILED: no dshow audio device found".to_string());
    }
    Ok(devices)
}

fn read_last_stderr_line_from_bytes(stderr: &[u8]) -> Option<String> {
    let buf = String::from_utf8_lossy(stderr);
    buf.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn probe_record_input_spec(ffmpeg: &Path, spec: &str) -> Result<(), String> {
    let null_sink = if cfg!(windows) { "NUL" } else { "-" };
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "dshow", "-i"])
        .arg(spec)
        .args(["-t", "0.15", "-f", "null", null_sink])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("probe spawn failed: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let tail = read_last_stderr_line_from_bytes(&output.stderr)
        .unwrap_or_else(|| "probe failed without stderr".to_string());
    Err(format!("{tail} (status={})", output.status))
}

fn score_audio_device_name(name: &str) -> i32 {
    let lower = name.to_lowercase();
    let mut score = 0_i32;
    for kw in ["microphone", "mic", "array", "input", "capture"] {
        if lower.contains(kw) {
            score += 10;
        }
    }
    if name.contains("麦克风") || name.contains("阵列") {
        score += 12;
    }
    for kw in [
        "headset",
        "headphone",
        "speaker",
        "output",
        "loopback",
        "stereo mix",
        "broadcast",
        "virtual",
    ] {
        if lower.contains(kw) {
            score -= 10;
        }
    }
    if name.contains("耳机") || name.contains("扬声器") {
        score -= 12;
    }
    score
}

fn attempt_auto_select(
    ffmpeg: &Path,
    devices: &[DshowDevice],
    strategy_used: InputStrategy,
) -> Result<ResolvedRecordInput, String> {
    let mut candidates: Vec<AutoCandidate> = devices
        .iter()
        .enumerate()
        .map(|(idx, d)| {
            let target = d
                .alternative_name
                .as_deref()
                .unwrap_or(d.name.as_str())
                .to_string();
            AutoCandidate {
                spec: format!("audio={target}"),
                display_name: d.name.clone(),
                score: score_audio_device_name(&target),
                order: idx,
            }
        })
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.order.cmp(&b.order)));

    let mut failures: Vec<String> = Vec::new();
    for cand in candidates {
        match probe_record_input_spec(ffmpeg, cand.spec.as_str()) {
            Ok(()) => {
                return Ok(ResolvedRecordInput {
                    spec: normalize_record_input_spec(cand.spec.as_str()),
                    strategy_used: strategy_used.as_str().to_string(),
                    endpoint_id: None,
                    friendly_name: Some(cand.display_name),
                });
            }
            Err(e) => failures.push(format!("{} => {e}", cand.display_name)),
        }
    }
    let summary = failures
        .into_iter()
        .take(3)
        .collect::<Vec<String>>()
        .join(" | ");
    Err(format!(
        "E_RECORD_INPUT_AUTO_RESOLVE_FAILED: no probeable dshow audio input ({summary})"
    ))
}

fn endpoint_to_dshow_spec(
    ffmpeg: &Path,
    endpoint: &AudioEndpointInfo,
    devices: &[DshowDevice],
) -> Result<String, String> {
    let mut ranked: Vec<(u8, usize)> = Vec::new();
    for (idx, d) in devices.iter().enumerate() {
        let mut best: Option<u8> = match_priority(endpoint.friendly_name.as_str(), d.name.as_str());
        if let Some(alt) = d.alternative_name.as_deref() {
            if let Some(p) = match_priority(endpoint.friendly_name.as_str(), alt) {
                best = Some(best.map(|old| old.min(p)).unwrap_or(p));
            }
        }
        if let Some(priority) = best {
            ranked.push((priority, idx));
        }
    }
    if ranked.is_empty() {
        return Err(format!(
            "E_RECORD_INPUT_MAP_FAILED: no dshow device matched endpoint friendly name=\"{}\"",
            endpoint.friendly_name
        ));
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut failures = Vec::new();
    for (_, idx) in ranked {
        let d = &devices[idx];
        let mut targets = Vec::new();
        if let Some(alt) = d.alternative_name.as_deref() {
            targets.push(alt.to_string());
        }
        targets.push(d.name.clone());
        targets.dedup();
        for target in targets {
            let spec = normalize_record_input_spec(format!("audio={target}").as_str());
            match probe_record_input_spec(ffmpeg, spec.as_str()) {
                Ok(()) => return Ok(spec),
                Err(e) => failures.push(format!("{target} => {e}")),
            }
        }
    }
    let summary = failures
        .into_iter()
        .take(3)
        .collect::<Vec<String>>()
        .join(" | ");
    Err(format!(
        "E_RECORD_INPUT_MAP_FAILED: matched endpoint but probe failed ({summary})"
    ))
}

fn attempt_follow_default(
    ffmpeg: &Path,
    role: DefaultRole,
    devices: &[DshowDevice],
    strategy_used: InputStrategy,
) -> Result<ResolvedRecordInput, String> {
    let endpoint = audio_devices_windows::get_default_capture_endpoint(role.to_windows_role())?;
    let spec = endpoint_to_dshow_spec(ffmpeg, &endpoint, devices)?;
    Ok(ResolvedRecordInput {
        spec,
        strategy_used: strategy_used.as_str().to_string(),
        endpoint_id: Some(endpoint.endpoint_id),
        friendly_name: Some(endpoint.friendly_name),
    })
}

fn attempt_fixed(
    ffmpeg: &Path,
    endpoint_id: &str,
    devices: &[DshowDevice],
    strategy_used: InputStrategy,
) -> Result<ResolvedRecordInput, String> {
    let endpoint = audio_devices_windows::get_capture_endpoint_by_id(endpoint_id)?;
    let spec = endpoint_to_dshow_spec(ffmpeg, &endpoint, devices)?;
    Ok(ResolvedRecordInput {
        spec,
        strategy_used: strategy_used.as_str().to_string(),
        endpoint_id: Some(endpoint.endpoint_id),
        friendly_name: Some(endpoint.friendly_name),
    })
}

fn attempt_last_working(settings: &Settings, ffmpeg: &Path) -> Result<ResolvedRecordInput, String> {
    let raw = settings
        .record_last_working_dshow_spec
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            settings
                .record_input_spec
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .ok_or_else(|| {
            "E_RECORD_INPUT_LAST_WORKING_MISSING: no last working dshow spec".to_string()
        })?;
    let spec = normalize_record_input_spec(raw);
    probe_record_input_spec(ffmpeg, spec.as_str())
        .map_err(|e| format!("E_RECORD_INPUT_LAST_WORKING_FAILED: {e}"))?;
    Ok(ResolvedRecordInput {
        spec,
        strategy_used: "last_working".to_string(),
        endpoint_id: settings.record_last_working_endpoint_id.clone(),
        friendly_name: settings.record_last_working_friendly_name.clone(),
    })
}

fn now_epoch_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(v) => v.as_millis() as i64,
        Err(_) => 0,
    }
}

fn save_last_working_cache(
    data_dir: &Path,
    settings: &mut Settings,
    resolved: &ResolvedRecordInput,
) -> Result<(), String> {
    let mut changed = false;
    let next_endpoint_id = resolved.endpoint_id.clone();
    let next_friendly_name = resolved.friendly_name.clone();
    let next_spec = Some(resolved.spec.clone());
    if settings.record_last_working_endpoint_id != next_endpoint_id {
        settings.record_last_working_endpoint_id = next_endpoint_id;
        changed = true;
    }
    if settings.record_last_working_friendly_name != next_friendly_name {
        settings.record_last_working_friendly_name = next_friendly_name;
        changed = true;
    }
    if settings.record_last_working_dshow_spec != next_spec {
        settings.record_last_working_dshow_spec = next_spec;
        changed = true;
    }
    let next_ts = Some(now_epoch_ms());
    if settings.record_last_working_ts_ms != next_ts {
        settings.record_last_working_ts_ms = next_ts;
        changed = true;
    }
    if !changed {
        return Ok(());
    }
    settings::save_settings(data_dir, settings)
        .map_err(|e| format!("E_RECORD_INPUT_CACHE_SAVE_FAILED: {e}"))
}

fn build_resolve_failed(strategy: InputStrategy, errors: &[String]) -> String {
    let summary = errors
        .iter()
        .take(3)
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(" | ");
    format!(
        "E_RECORD_INPUT_RESOLVE_FAILED: strategy={} failed ({summary})",
        strategy.as_str()
    )
}

pub fn resolve_record_input_for_recording(
    data_dir: &Path,
    ffmpeg_cmd: &str,
) -> Result<ResolvedRecordInput, String> {
    let ffmpeg = Path::new(ffmpeg_cmd);
    let mut settings = settings::load_settings_strict(data_dir).map_err(|e| e.to_string())?;
    let strategy = parse_strategy(&settings)?;
    let role = parse_default_role(&settings)?;
    let dshow_devices = list_dshow_audio_devices(ffmpeg)?;
    let mut errors = Vec::new();

    let resolved = match strategy {
        InputStrategy::FixedDevice => {
            let mut resolved: Option<ResolvedRecordInput> = None;
            if let Some(id) = settings
                .record_fixed_endpoint_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                match attempt_fixed(ffmpeg, id, &dshow_devices, strategy) {
                    Ok(v) => resolved = Some(v),
                    Err(e) => errors.push(e),
                }
            } else {
                errors.push("E_RECORD_INPUT_FIXED_MISSING: record_fixed_endpoint_id is required when record_input_strategy=fixed_device".to_string());
            }
            if resolved.is_none() {
                match attempt_follow_default(ffmpeg, role, &dshow_devices, strategy) {
                    Ok(v) => resolved = Some(v),
                    Err(e) => errors.push(e),
                }
            }
            if resolved.is_none() {
                match attempt_auto_select(ffmpeg, &dshow_devices, strategy) {
                    Ok(v) => resolved = Some(v),
                    Err(e) => errors.push(e),
                }
            }
            resolved.ok_or_else(|| build_resolve_failed(strategy, &errors))?
        }
        InputStrategy::FollowDefault => {
            let mut resolved: Option<ResolvedRecordInput> = None;
            match attempt_follow_default(ffmpeg, role, &dshow_devices, strategy) {
                Ok(v) => resolved = Some(v),
                Err(e) => errors.push(e),
            }
            if resolved.is_none() {
                match attempt_last_working(&settings, ffmpeg) {
                    Ok(v) => resolved = Some(v),
                    Err(e) => errors.push(e),
                }
            }
            if resolved.is_none() {
                match attempt_auto_select(ffmpeg, &dshow_devices, strategy) {
                    Ok(v) => resolved = Some(v),
                    Err(e) => errors.push(e),
                }
            }
            resolved.ok_or_else(|| build_resolve_failed(strategy, &errors))?
        }
        InputStrategy::AutoSelect => attempt_auto_select(ffmpeg, &dshow_devices, strategy)
            .map_err(|e| {
                errors.push(e);
                build_resolve_failed(strategy, &errors)
            })?,
    };

    let _ = save_last_working_cache(data_dir, &mut settings, &resolved);
    Ok(resolved)
}

pub fn list_audio_capture_devices_for_settings() -> Result<Vec<AudioCaptureDeviceView>, String> {
    let mut devices = audio_devices_windows::list_active_capture_endpoints()?;
    devices.sort_by(|a, b| a.friendly_name.cmp(&b.friendly_name));
    let default_comm =
        audio_devices_windows::get_default_capture_endpoint(DefaultCaptureRole::Communications)
            .ok()
            .map(|v| v.endpoint_id);
    let default_console =
        audio_devices_windows::get_default_capture_endpoint(DefaultCaptureRole::Console)
            .ok()
            .map(|v| v.endpoint_id);

    Ok(devices
        .into_iter()
        .map(|item| AudioCaptureDeviceView {
            is_default_communications: default_comm
                .as_deref()
                .map(|id| id == item.endpoint_id)
                .unwrap_or(false),
            is_default_console: default_console
                .as_deref()
                .map(|id| id == item.endpoint_id)
                .unwrap_or(false),
            endpoint_id: item.endpoint_id,
            friendly_name: item.friendly_name,
        })
        .collect())
}

pub fn normalize_strategy_for_settings(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        STRATEGY_FOLLOW_DEFAULT => Some(STRATEGY_FOLLOW_DEFAULT),
        STRATEGY_FIXED_DEVICE => Some(STRATEGY_FIXED_DEVICE),
        STRATEGY_AUTO_SELECT => Some(STRATEGY_AUTO_SELECT),
        _ => None,
    }
}

pub fn normalize_default_role_for_settings(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        ROLE_COMMUNICATIONS | "ecommunications" => Some(ROLE_COMMUNICATIONS),
        ROLE_CONSOLE | "econsole" => Some(ROLE_CONSOLE),
        _ => None,
    }
}

pub fn default_strategy() -> &'static str {
    STRATEGY_FOLLOW_DEFAULT
}

pub fn default_role() -> &'static str {
    ROLE_COMMUNICATIONS
}

#[cfg(test)]
mod tests {
    use super::{
        collapse_ws_lower, match_priority, normalize_default_role_for_settings,
        normalize_strategy_for_settings,
    };

    #[test]
    fn normalize_strategy_and_role() {
        assert_eq!(
            normalize_strategy_for_settings("follow_default"),
            Some("follow_default")
        );
        assert_eq!(
            normalize_strategy_for_settings("fixed_device"),
            Some("fixed_device")
        );
        assert_eq!(
            normalize_strategy_for_settings("auto_select"),
            Some("auto_select")
        );
        assert_eq!(normalize_strategy_for_settings("x"), None);
        assert_eq!(
            normalize_default_role_for_settings("communications"),
            Some("communications")
        );
        assert_eq!(
            normalize_default_role_for_settings("eConsole"),
            Some("console")
        );
        assert_eq!(normalize_default_role_for_settings("x"), None);
    }

    #[test]
    fn name_match_priority_behaves() {
        assert_eq!(collapse_ws_lower("USB   MIC"), "usb mic");
        assert_eq!(match_priority("USB Mic", "usb mic"), Some(0));
        assert_eq!(match_priority("USB   Mic", "usb mic"), Some(1));
        assert_eq!(
            match_priority("USB Microphone Array", "microphone"),
            Some(2)
        );
        assert_eq!(match_priority("USB Mic", "Speaker"), None);
    }
}
