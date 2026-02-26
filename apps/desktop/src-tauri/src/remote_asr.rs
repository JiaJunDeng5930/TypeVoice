use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, Result};
use reqwest::{multipart, Client};
use serde::Deserialize;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::llm::ApiKeyStatus;
use crate::trace::Span;

const KEYRING_SERVICE: &str = "typevoice";
const KEYRING_USER: &str = "remote_asr_api_key";
const API_KEY_ENV: &str = "TYPEVOICE_REMOTE_ASR_API_KEY";
const DEFAULT_SLICE_SEC: f64 = 60.0;
const DEFAULT_OVERLAP_SEC: f64 = 0.5;
const MAX_DEDUPE_CHARS: usize = 64;

#[derive(Debug, Clone)]
pub struct RemoteAsrConfig {
    pub url: String,
    pub model: Option<String>,
    pub concurrency: usize,
}

#[derive(Debug, Clone)]
pub struct RemoteAsrMetrics {
    pub audio_seconds: f64,
    pub elapsed_ms: i64,
    pub rtf: f64,
    pub slice_count: usize,
    pub concurrency_used: usize,
    pub model_id: String,
    pub model_version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteAsrOutput {
    pub text: String,
    pub metrics: RemoteAsrMetrics,
}

#[derive(Debug, Clone)]
pub struct RemoteAsrError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for RemoteAsrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RemoteAsrError {}

#[derive(Debug, Clone)]
struct WavInfo {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    block_align: u16,
    data_offset: usize,
    data_len: usize,
    duration_seconds: f64,
}

#[derive(Debug, Clone)]
struct SliceRequest {
    index: usize,
    wav_bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct RemoteResp {
    text: Option<String>,
}

fn err(code: &str, message: impl Into<String>) -> RemoteAsrError {
    RemoteAsrError {
        code: code.to_string(),
        message: message.into(),
    }
}

pub fn set_api_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| anyhow!("{e:?}"))?;
    entry.set_password(key).map_err(|e| anyhow!("{e:?}"))?;
    Ok(())
}

pub fn clear_api_key() -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| anyhow!("{e:?}"))?;
    let _ = entry.set_password("").map_err(|e| anyhow!("{e:?}"));
    Ok(())
}

pub fn api_key_status() -> ApiKeyStatus {
    if let Ok(k) = std::env::var(API_KEY_ENV) {
        if !k.trim().is_empty() {
            return ApiKeyStatus {
                configured: true,
                source: "env".to_string(),
                reason: None,
            };
        }
    }
    let entry = match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        Ok(v) => v,
        Err(e) => {
            return ApiKeyStatus {
                configured: false,
                source: "keyring".to_string(),
                reason: Some(format!("keyring_entry_init_failed:{e:?}")),
            };
        }
    };
    match entry.get_password() {
        Ok(k) if !k.trim().is_empty() => ApiKeyStatus {
            configured: true,
            source: "keyring".to_string(),
            reason: None,
        },
        Ok(_) => ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some("empty".to_string()),
        },
        Err(e) => ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some(format!("keyring_get_failed:{e:?}")),
        },
    }
}

fn load_api_key() -> Result<String, RemoteAsrError> {
    if let Ok(v) = std::env::var(API_KEY_ENV) {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| {
        err(
            "E_REMOTE_ASR_API_KEY_MISSING",
            format!("keyring init failed: {e:?}"),
        )
    })?;
    let v = entry.get_password().map_err(|e| {
        err(
            "E_REMOTE_ASR_API_KEY_MISSING",
            format!("keyring get failed: {e:?}"),
        )
    })?;
    if v.trim().is_empty() {
        return Err(err(
            "E_REMOTE_ASR_API_KEY_MISSING",
            "remote ASR API key is empty",
        ));
    }
    Ok(v)
}

pub async fn transcribe_remote(
    data_dir: &Path,
    task_id: &str,
    wav_path: &Path,
    token: &CancellationToken,
    cfg: &RemoteAsrConfig,
) -> Result<RemoteAsrOutput, RemoteAsrError> {
    let span = Span::start(
        data_dir,
        Some(task_id),
        "Transcribe",
        "ASR.remote_transcribe",
        Some(serde_json::json!({
            "url": cfg.url,
            "has_model": cfg.model.as_deref().map(|v| !v.is_empty()).unwrap_or(false),
            "concurrency": cfg.concurrency,
            "slice_sec": DEFAULT_SLICE_SEC,
            "overlap_sec": DEFAULT_OVERLAP_SEC,
        })),
    );

    let out = transcribe_remote_inner(wav_path, token, cfg).await;
    match &out {
        Ok(v) => span.ok(Some(serde_json::json!({
            "slice_count": v.metrics.slice_count,
            "concurrency_used": v.metrics.concurrency_used,
            "elapsed_ms": v.metrics.elapsed_ms,
            "rtf": v.metrics.rtf,
            "audio_seconds": v.metrics.audio_seconds,
        }))),
        Err(e) => span.err("remote", &e.code, &e.message, None),
    }
    out
}

async fn transcribe_remote_inner(
    wav_path: &Path,
    token: &CancellationToken,
    cfg: &RemoteAsrConfig,
) -> Result<RemoteAsrOutput, RemoteAsrError> {
    if token.is_cancelled() {
        return Err(err("E_CANCELLED", "cancelled"));
    }
    let url = cfg.url.trim();
    if url.is_empty() {
        return Err(err("E_REMOTE_ASR_CONFIG", "remote_asr_url is required"));
    }
    if cfg.concurrency == 0 {
        return Err(err(
            "E_REMOTE_ASR_CONFIG",
            "remote_asr_concurrency must be >= 1",
        ));
    }

    let key = load_api_key()?;
    let bytes = tokio::fs::read(wav_path)
        .await
        .map_err(|e| err("E_REMOTE_ASR_WAV_READ", format!("read wav failed: {e}")))?;
    let wav = parse_wav(&bytes)?;
    let slices = build_slice_requests(&bytes, &wav, DEFAULT_SLICE_SEC, DEFAULT_OVERLAP_SEC)?;
    if slices.is_empty() {
        return Err(err(
            "E_REMOTE_ASR_WAV_UNSUPPORTED",
            "wav has no audio samples",
        ));
    }

    let client = Client::new();
    let concurrency_used = cfg.concurrency.min(slices.len()).max(1);
    let mut parts = vec![String::new(); slices.len()];
    let mut set = JoinSet::new();
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency_used));
    let started = Instant::now();

    for slice in slices {
        let client2 = client.clone();
        let key2 = key.clone();
        let model2 = cfg.model.clone();
        let url2 = url.to_string();
        let token2 = token.clone();
        let semaphore2 = semaphore.clone();
        set.spawn(async move {
            let _permit = semaphore2
                .acquire_owned()
                .await
                .map_err(|_| err("E_REMOTE_ASR_INTERNAL", "semaphore closed"))?;
            if token2.is_cancelled() {
                return Err(err("E_CANCELLED", "cancelled"));
            }
            transcribe_one_slice(&client2, &url2, &key2, model2.as_deref(), slice, &token2).await
        });
    }

    let mut completed = 0usize;
    while completed < parts.len() {
        let next = tokio::select! {
            _ = token.cancelled() => {
                set.abort_all();
                return Err(err("E_CANCELLED", "cancelled"));
            }
            v = set.join_next() => v
        };
        match next {
            Some(Ok(Ok((index, text)))) => {
                parts[index] = text;
                completed += 1;
            }
            Some(Ok(Err(e))) => {
                set.abort_all();
                return Err(e);
            }
            Some(Err(e)) => {
                set.abort_all();
                return Err(err(
                    "E_REMOTE_ASR_INTERNAL",
                    format!("slice task join failed: {e}"),
                ));
            }
            None => break,
        }
    }

    if completed != parts.len() {
        return Err(err(
            "E_REMOTE_ASR_INTERNAL",
            format!(
                "slice completion mismatch: expected={}, got={completed}",
                parts.len()
            ),
        ));
    }

    let text = merge_slices(&parts);
    if text.trim().is_empty() {
        return Err(err("E_REMOTE_ASR_EMPTY_TEXT", "merged text is empty"));
    }

    let elapsed_ms = started.elapsed().as_millis() as i64;
    let audio_seconds = wav.duration_seconds;
    let rtf = (elapsed_ms as f64 / 1000.0) / audio_seconds.max(1e-6);
    Ok(RemoteAsrOutput {
        text,
        metrics: RemoteAsrMetrics {
            audio_seconds,
            elapsed_ms,
            rtf,
            slice_count: parts.len(),
            concurrency_used,
            model_id: cfg
                .model
                .clone()
                .unwrap_or_else(|| "remote/transcribe".to_string()),
            model_version: None,
        },
    })
}

async fn transcribe_one_slice(
    client: &Client,
    url: &str,
    key: &str,
    model: Option<&str>,
    slice: SliceRequest,
    token: &CancellationToken,
) -> Result<(usize, String), RemoteAsrError> {
    let part = multipart::Part::bytes(slice.wav_bytes)
        .file_name(format!("segment_{}.wav", slice.index))
        .mime_str("audio/wav")
        .map_err(|e| err("E_REMOTE_ASR_CONFIG", format!("invalid mime: {e}")))?;
    let mut form = multipart::Form::new().part("file", part);
    if let Some(m) = model {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            form = form.text("model", trimmed.to_string());
        }
    }

    let req = client
        .post(url.to_string())
        .bearer_auth(key)
        .multipart(form)
        .send();
    let resp = tokio::select! {
        _ = token.cancelled() => return Err(err("E_CANCELLED", "cancelled")),
        v = req => v
    }
    .map_err(|e| err("E_REMOTE_ASR_HTTP_SEND", format!("request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| err("E_REMOTE_ASR_PARSE", format!("read response failed: {e}")))?;

    if !status.is_success() {
        let code = format!("E_REMOTE_ASR_HTTP_STATUS_{}", status.as_u16());
        let msg = if body.len() > 512 {
            format!("{}...(truncated)", &body[..512])
        } else {
            body
        };
        return Err(err(&code, msg));
    }

    let parsed: RemoteResp = serde_json::from_str(&body)
        .map_err(|e| err("E_REMOTE_ASR_PARSE", format!("invalid json response: {e}")))?;
    let text = parsed.text.unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Err(err(
            "E_REMOTE_ASR_EMPTY_TEXT",
            "response.text is missing or empty",
        ));
    }
    Ok((slice.index, text))
}

fn parse_wav(bytes: &[u8]) -> Result<WavInfo, RemoteAsrError> {
    if bytes.len() < 12 {
        return Err(err("E_REMOTE_ASR_WAV_UNSUPPORTED", "wav header too short"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err("E_REMOTE_ASR_WAV_UNSUPPORTED", "not a RIFF/WAVE file"));
    }

    let mut pos = 12usize;
    let mut channels = None;
    let mut sample_rate = None;
    let mut bits_per_sample = None;
    let mut block_align = None;
    let mut data_offset = None;
    let mut data_len = None;
    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_size = le_u32(bytes, pos + 4)? as usize;
        let data_start = pos + 8;
        let data_end = data_start.saturating_add(chunk_size);
        if data_end > bytes.len() {
            return Err(err(
                "E_REMOTE_ASR_WAV_UNSUPPORTED",
                "wav chunk out of bounds",
            ));
        }

        if chunk_id == b"fmt " {
            if chunk_size < 16 {
                return Err(err("E_REMOTE_ASR_WAV_UNSUPPORTED", "fmt chunk too short"));
            }
            let audio_format = le_u16(bytes, data_start)?;
            let ch = le_u16(bytes, data_start + 2)?;
            let sr = le_u32(bytes, data_start + 4)?;
            let ba = le_u16(bytes, data_start + 12)?;
            let bps = le_u16(bytes, data_start + 14)?;
            if audio_format != 1 {
                return Err(err(
                    "E_REMOTE_ASR_WAV_UNSUPPORTED",
                    format!("only PCM is supported, got audio_format={audio_format}"),
                ));
            }
            channels = Some(ch);
            sample_rate = Some(sr);
            block_align = Some(ba);
            bits_per_sample = Some(bps);
        } else if chunk_id == b"data" && data_offset.is_none() {
            data_offset = Some(data_start);
            data_len = Some(chunk_size);
        }

        let pad = if chunk_size % 2 == 1 { 1 } else { 0 };
        pos = data_end.saturating_add(pad);
    }

    let channels =
        channels.ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "missing fmt chunk"))?;
    let sample_rate =
        sample_rate.ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "missing sample_rate"))?;
    let bits_per_sample = bits_per_sample
        .ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "missing bits_per_sample"))?;
    let block_align =
        block_align.ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "missing block_align"))?;
    let data_offset =
        data_offset.ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "missing data chunk"))?;
    let data_len =
        data_len.ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "missing data length"))?;

    if channels != 1 || sample_rate != 16_000 || bits_per_sample != 16 {
        return Err(err(
            "E_REMOTE_ASR_WAV_UNSUPPORTED",
            format!(
                "expected mono/16k/16-bit wav, got channels={channels}, sample_rate={sample_rate}, bits={bits_per_sample}"
            ),
        ));
    }
    if block_align == 0 {
        return Err(err(
            "E_REMOTE_ASR_WAV_UNSUPPORTED",
            "block_align must be > 0",
        ));
    }
    let bytes_per_sec = sample_rate as usize * block_align as usize;
    if bytes_per_sec == 0 {
        return Err(err(
            "E_REMOTE_ASR_WAV_UNSUPPORTED",
            "bytes_per_sec must be > 0",
        ));
    }
    let duration_seconds = data_len as f64 / bytes_per_sec as f64;
    Ok(WavInfo {
        channels,
        sample_rate,
        bits_per_sample,
        block_align,
        data_offset,
        data_len,
        duration_seconds,
    })
}

fn build_slice_requests(
    source: &[u8],
    wav: &WavInfo,
    slice_sec: f64,
    overlap_sec: f64,
) -> Result<Vec<SliceRequest>, RemoteAsrError> {
    if wav.duration_seconds <= 0.0 {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    let mut index = 0usize;
    let mut base_start = 0.0_f64;
    while base_start < wav.duration_seconds {
        let base_end = (base_start + slice_sec).min(wav.duration_seconds);
        let start = if index == 0 {
            base_start
        } else {
            (base_start - overlap_sec).max(0.0)
        };
        let end = if base_end >= wav.duration_seconds {
            wav.duration_seconds
        } else {
            (base_end + overlap_sec).min(wav.duration_seconds)
        };
        let data = extract_segment_pcm(source, wav, start, end)?;
        if !data.is_empty() {
            let wav_bytes = build_wav_bytes(
                &data,
                wav.channels,
                wav.sample_rate,
                wav.bits_per_sample,
                wav.block_align,
            );
            out.push(SliceRequest { index, wav_bytes });
        }
        index += 1;
        base_start += slice_sec;
    }
    Ok(out)
}

fn extract_segment_pcm(
    source: &[u8],
    wav: &WavInfo,
    start_sec: f64,
    end_sec: f64,
) -> Result<Vec<u8>, RemoteAsrError> {
    if end_sec <= start_sec {
        return Ok(Vec::new());
    }
    let samples_start = (start_sec * wav.sample_rate as f64).floor().max(0.0) as usize;
    let samples_end = (end_sec * wav.sample_rate as f64).ceil().max(0.0) as usize;
    let mut byte_start = samples_start.saturating_mul(wav.block_align as usize);
    let mut byte_end = samples_end.saturating_mul(wav.block_align as usize);
    byte_start = byte_start.min(wav.data_len);
    byte_end = byte_end.min(wav.data_len);
    if byte_end <= byte_start {
        return Ok(Vec::new());
    }
    let abs_start = wav.data_offset + byte_start;
    let abs_end = wav.data_offset + byte_end;
    if abs_end > source.len() || abs_start > abs_end {
        return Err(err(
            "E_REMOTE_ASR_WAV_UNSUPPORTED",
            "segment range out of bounds",
        ));
    }
    Ok(source[abs_start..abs_end].to_vec())
}

fn build_wav_bytes(
    pcm_data: &[u8],
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    block_align: u16,
) -> Vec<u8> {
    let byte_rate = sample_rate * block_align as u32;
    let data_len = pcm_data.len() as u32;
    let riff_len = 36u32 + data_len;
    let mut out = Vec::with_capacity((44 + pcm_data.len()).max(44));
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm_data);
    out
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, RemoteAsrError> {
    let end = offset.saturating_add(2);
    let src = bytes
        .get(offset..end)
        .ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "u16 read out of bounds"))?;
    Ok(u16::from_le_bytes([src[0], src[1]]))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, RemoteAsrError> {
    let end = offset.saturating_add(4);
    let src = bytes
        .get(offset..end)
        .ok_or_else(|| err("E_REMOTE_ASR_WAV_UNSUPPORTED", "u32 read out of bounds"))?;
    Ok(u32::from_le_bytes([src[0], src[1], src[2], src[3]]))
}

fn merge_slices(parts: &[String]) -> String {
    let mut merged = String::new();
    for part in parts {
        let chunk = part.trim();
        if chunk.is_empty() {
            continue;
        }
        if merged.is_empty() {
            merged.push_str(chunk);
            continue;
        }
        let overlap = longest_overlap_chars(&merged, chunk, MAX_DEDUPE_CHARS);
        let trimmed = skip_first_chars(chunk, overlap);
        if trimmed.is_empty() {
            continue;
        }
        let need_space = needs_space_between(&merged, &trimmed);
        if need_space {
            merged.push(' ');
        }
        merged.push_str(&trimmed);
    }
    merged
}

fn needs_space_between(left: &str, right: &str) -> bool {
    let left_tail = left.chars().last();
    let right_head = right.chars().next();
    match (left_tail, right_head) {
        (Some(a), Some(b)) => a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric(),
        _ => false,
    }
}

fn longest_overlap_chars(left: &str, right: &str, max_chars: usize) -> usize {
    let left_count = left.chars().count();
    let right_count = right.chars().count();
    let max_k = left_count.min(right_count).min(max_chars);
    for k in (1..=max_k).rev() {
        if take_last_chars(left, k) == take_first_chars(right, k) {
            return k;
        }
    }
    0
}

fn take_last_chars(s: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return chars.into_iter().collect();
    }
    chars[chars.len() - n..].iter().collect()
}

fn take_first_chars(s: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    s.chars().take(n).collect()
}

fn skip_first_chars(s: &str, n: usize) -> String {
    if n == 0 {
        return s.to_string();
    }
    s.chars().skip(n).collect()
}

#[cfg(test)]
mod tests {
    use super::{merge_slices, parse_wav};

    fn build_test_wav(seconds: usize) -> Vec<u8> {
        let sample_rate = 16_000u32;
        let channels = 1u16;
        let bits = 16u16;
        let block_align = channels * (bits / 8);
        let total_samples = seconds * sample_rate as usize;
        let pcm = vec![0u8; total_samples * block_align as usize];
        let byte_rate = sample_rate * block_align as u32;
        let data_len = pcm.len() as u32;
        let riff_len = 36u32 + data_len;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_len.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.extend_from_slice(&pcm);
        out
    }

    #[test]
    fn parse_wav_accepts_mono_16k_16bit() {
        let wav = build_test_wav(2);
        let info = parse_wav(&wav).expect("parse");
        assert_eq!(info.channels, 1);
        assert_eq!(info.sample_rate, 16_000);
        assert_eq!(info.bits_per_sample, 16);
        assert!(info.duration_seconds >= 1.99);
    }

    #[test]
    fn merge_slices_dedupes_overlap() {
        let merged = merge_slices(&[
            "hello world this is".to_string(),
            "this is a test".to_string(),
            "a test for remote asr".to_string(),
        ]);
        assert_eq!(merged, "hello world this is a test for remote asr");
    }
}
