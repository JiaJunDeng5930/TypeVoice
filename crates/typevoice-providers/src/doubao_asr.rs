use std::io::Write;

use anyhow::{anyhow, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::llm::ApiKeyStatus;

const KEYRING_SERVICE: &str = "typevoice";
const APP_KEY_USER: &str = "doubao_asr_app_key";
const ACCESS_KEY_USER: &str = "doubao_asr_access_key";
const APP_KEY_ENV: &str = "TYPEVOICE_DOUBAO_ASR_APP_KEY";
const ACCESS_KEY_ENV: &str = "TYPEVOICE_DOUBAO_ASR_ACCESS_KEY";
pub const PCM_SAMPLE_RATE: u32 = 16_000;
pub const PCM_CHANNELS: u16 = 1;
pub const PCM_BITS: u16 = 16;
pub const DOUBAO_WS_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";
pub const DOUBAO_RESOURCE_ID: &str = "volc.seedasr.sauc.duration";

#[derive(Debug, Clone)]
pub struct DoubaoCredentials {
    pub app_key: String,
    pub access_key: String,
}

pub struct DoubaoServerPayload {
    pub value: Value,
    pub is_last: bool,
}

pub fn set_credentials(app_key: &str, access_key: &str) -> Result<()> {
    let app_key = app_key.trim();
    let access_key = access_key.trim();
    if app_key.is_empty() {
        return Err(anyhow!("E_DOUBAO_ASR_APP_KEY_MISSING: app key is required"));
    }
    if access_key.is_empty() {
        return Err(anyhow!(
            "E_DOUBAO_ASR_ACCESS_KEY_MISSING: access key is required"
        ));
    }
    keyring::Entry::new(KEYRING_SERVICE, APP_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password(app_key)
        .map_err(|e| anyhow!("{e:?}"))?;
    keyring::Entry::new(KEYRING_SERVICE, ACCESS_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password(access_key)
        .map_err(|e| anyhow!("{e:?}"))?;
    Ok(())
}

pub fn clear_credentials() -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, APP_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password("")
        .map_err(|e| anyhow!("{e:?}"))?;
    keyring::Entry::new(KEYRING_SERVICE, ACCESS_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password("")
        .map_err(|e| anyhow!("{e:?}"))?;
    Ok(())
}

pub fn credentials_status() -> ApiKeyStatus {
    if env_credentials().is_some() {
        return ApiKeyStatus {
            configured: true,
            source: "env".to_string(),
            reason: None,
        };
    }
    match keyring_credentials() {
        Ok(Some(_)) => ApiKeyStatus {
            configured: true,
            source: "keyring".to_string(),
            reason: None,
        },
        Ok(None) => ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some("empty".to_string()),
        },
        Err(e) => ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some(e.to_string()),
        },
    }
}

pub fn load_credentials() -> Result<DoubaoCredentials> {
    if let Some(v) = env_credentials() {
        return Ok(v);
    }
    match keyring_credentials()? {
        Some(v) => Ok(v),
        None => Err(anyhow!(
            "E_DOUBAO_ASR_CREDENTIALS_MISSING: doubao ASR credentials are missing"
        )),
    }
}

pub async fn check_credentials_live() -> Result<()> {
    let creds = load_credentials()?;
    let req = build_websocket_request(&creds)?;
    let (ws, _) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .map_err(|_| anyhow!("E_DOUBAO_ASR_CHECK_TIMEOUT: connection timed out"))?
    .context("E_DOUBAO_ASR_CHECK_CONNECT: connect doubao websocket failed")?;

    let (mut write, mut read) = ws.split();
    write
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            build_full_client_request_frame()?,
        ))
        .await
        .context("E_DOUBAO_ASR_CHECK_INIT: send doubao init frame failed")?;

    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(10), read.next())
            .await
            .map_err(|_| anyhow!("E_DOUBAO_ASR_CHECK_TIMEOUT: response timed out"))?
            .ok_or_else(|| anyhow!("E_DOUBAO_ASR_CHECK_EMPTY: websocket closed without response"))?
            .context("E_DOUBAO_ASR_CHECK_READ: read doubao websocket message failed")?;
        if msg.is_binary() {
            parse_server_payload(&msg.into_data())?;
            break;
        }
        if msg.is_close() {
            return Err(anyhow!(
                "E_DOUBAO_ASR_CHECK_EMPTY: websocket closed without binary response"
            ));
        }
    }
    write.close().await.ok();
    Ok(())
}

pub fn build_websocket_request(
    creds: &DoubaoCredentials,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let mut req = DOUBAO_WS_URL
        .into_client_request()
        .context("build doubao websocket request failed")?;
    req.headers_mut().insert(
        "X-Api-App-Key",
        creds
            .app_key
            .parse()
            .context("invalid doubao app key header")?,
    );
    req.headers_mut().insert(
        "X-Api-Access-Key",
        creds
            .access_key
            .parse()
            .context("invalid doubao access key header")?,
    );
    req.headers_mut()
        .insert("X-Api-Resource-Id", DOUBAO_RESOURCE_ID.parse().unwrap());
    req.headers_mut().insert(
        "X-Api-Connect-Id",
        uuid::Uuid::new_v4().to_string().parse()?,
    );
    Ok(req)
}

pub fn build_full_client_request_frame() -> Result<Vec<u8>> {
    let payload = serde_json::json!({
        "user": {"uid": "typevoice"},
        "audio": {
            "format": "pcm",
            "codec": "raw",
            "rate": PCM_SAMPLE_RATE,
            "bits": PCM_BITS,
            "channel": PCM_CHANNELS,
        },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,
            "enable_punc": true,
            "show_utterances": true,
            "result_type": "full",
        },
    });
    let compressed = gzip(serde_json::to_string(&payload)?.as_bytes())?;
    let mut out = vec![0x11, 0x11, 0x11, 0x00];
    out.extend_from_slice(&1_i32.to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

pub fn build_audio_frame(sequence: u64, pcm: &[u8], is_last: bool) -> Result<Vec<u8>> {
    let compressed = gzip(pcm)?;
    let seq = if is_last {
        -(sequence as i32)
    } else {
        sequence as i32
    };
    let flags = if is_last { 0x03 } else { 0x01 };
    let mut out = vec![0x11, (0x02 << 4) | flags, 0x01, 0x00];
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

pub fn parse_server_payload(frame: &[u8]) -> Result<DoubaoServerPayload> {
    if frame.len() < 8 {
        return Err(anyhow!("doubao response frame too short"));
    }
    let header_size = ((frame[0] & 0x0f) as usize) * 4;
    if frame.len() < header_size + 4 {
        return Err(anyhow!("doubao response frame header invalid"));
    }
    let msg_type = frame[1] >> 4;
    let flags = frame[1] & 0x0f;
    let compression = frame[2] & 0x0f;
    let mut offset = header_size;
    let mut is_last = false;
    if flags == 0x01 || flags == 0x03 {
        if frame.len() < offset + 4 {
            return Err(anyhow!("doubao response sequence missing"));
        }
        let sequence = i32::from_be_bytes(frame[offset..offset + 4].try_into().unwrap());
        is_last = sequence < 0;
        offset += 4;
    }
    if msg_type == 0x0f {
        if frame.len() < offset + 8 {
            return Err(anyhow!("doubao error frame invalid"));
        }
        let code = u32::from_be_bytes(frame[offset..offset + 4].try_into().unwrap());
        let size = u32::from_be_bytes(frame[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start.saturating_add(size).min(frame.len());
        let message = String::from_utf8_lossy(&frame[start..end]).to_string();
        return Err(anyhow!("E_DOUBAO_ASR_ERROR_{code}: {message}"));
    }
    if frame.len() < offset + 4 {
        return Err(anyhow!("doubao response payload missing"));
    }
    let size = u32::from_be_bytes(frame[offset..offset + 4].try_into().unwrap()) as usize;
    let start = offset + 4;
    let end = start.saturating_add(size);
    if end > frame.len() {
        return Err(anyhow!("doubao response payload out of bounds"));
    }
    let bytes = if compression == 0x01 {
        gunzip(&frame[start..end])?
    } else {
        frame[start..end].to_vec()
    };
    Ok(DoubaoServerPayload {
        value: serde_json::from_slice(&bytes).context("parse doubao response json failed")?,
        is_last,
    })
}

pub fn extract_text(payload: &Value) -> Option<String> {
    payload
        .get("result")
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

fn env_credentials() -> Option<DoubaoCredentials> {
    let app_key = std::env::var(APP_KEY_ENV).ok()?.trim().to_string();
    let access_key = std::env::var(ACCESS_KEY_ENV).ok()?.trim().to_string();
    if app_key.is_empty() || access_key.is_empty() {
        return None;
    }
    Some(DoubaoCredentials {
        app_key,
        access_key,
    })
}

fn keyring_credentials() -> Result<Option<DoubaoCredentials>> {
    let app_key = keyring::Entry::new(KEYRING_SERVICE, APP_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .get_password()
        .unwrap_or_default()
        .trim()
        .to_string();
    let access_key = keyring::Entry::new(KEYRING_SERVICE, ACCESS_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .get_password()
        .unwrap_or_default()
        .trim()
        .to_string();
    if app_key.is_empty() || access_key.is_empty() {
        return Ok(None);
    }
    Ok(Some(DoubaoCredentials {
        app_key,
        access_key,
    }))
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut dec = GzDecoder::new(bytes);
    let mut out = Vec::new();
    std::io::copy(&mut dec, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_frame_marks_last_sequence_negative() {
        let frame = build_audio_frame(3, &[1, 2, 3, 4], true).expect("frame");
        assert_eq!(frame[1] & 0x0f, 0x03);
        let seq = i32::from_be_bytes(frame[4..8].try_into().unwrap());
        assert_eq!(seq, -3);
    }

    #[test]
    fn server_payload_marks_negative_sequence_as_last() {
        let compressed =
            gzip(br#"{"result":{"text":"hello"}}"#).expect("response payload compresses");
        let mut frame = vec![0x11, 0x13, 0x01, 0x00];
        frame.extend_from_slice(&(-1_i32).to_be_bytes());
        frame.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        frame.extend_from_slice(&compressed);

        let parsed = parse_server_payload(&frame).expect("response parses");

        assert!(parsed.is_last);
        assert_eq!(extract_text(&parsed.value).as_deref(), Some("hello"));
    }
}
