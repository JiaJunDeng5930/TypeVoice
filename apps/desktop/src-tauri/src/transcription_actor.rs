use std::{
    collections::HashSet,
    io::Write,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_util::sync::CancellationToken;

use crate::{
    asr_service::AsrService,
    data_dir, doubao_asr, history, obs,
    settings::{self, Settings},
    transcription::{TranscriptionMetrics, TranscriptionResult},
    ui_events::{UiEvent, UiEventMailbox, UiEventStatus},
};

const LOCAL_CHUNK_MS: u64 = 60_000;
const DOUBAO_CHUNK_MS: u64 = 200;
const PCM_SAMPLE_RATE: u32 = 16_000;
const PCM_CHANNELS: u16 = 1;
const PCM_BITS: u16 = 16;
const DOUBAO_WS_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";
const DOUBAO_RESOURCE_ID: &str = "volc.seedasr.sauc.duration";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingProviderKind {
    Local,
    Remote,
    Doubao,
}

impl StreamingProviderKind {
    fn from_settings(s: &Settings) -> Self {
        match settings::resolve_asr_provider(s).as_str() {
            "doubao" => Self::Doubao,
            "remote" => Self::Remote,
            _ => Self::Local,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Doubao => "doubao",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamingSessionConfig {
    pub provider: StreamingProviderKind,
    pub chunk_ms: u64,
    pub chunk_bytes: usize,
}

#[derive(Debug)]
enum ActorMessage {
    Start {
        task_id: String,
        config: StreamingSessionConfig,
    },
    AudioChunk {
        task_id: String,
        sequence: u64,
        pcm: Vec<u8>,
        is_last: bool,
    },
    Finish {
        task_id: String,
    },
    Cancel {
        task_id: String,
    },
}

#[derive(Clone)]
pub struct TranscriptionActor {
    tx: mpsc::Sender<ActorMessage>,
    started: Arc<Mutex<HashSet<String>>>,
}

impl TranscriptionActor {
    pub fn new(mailbox: UiEventMailbox) -> Self {
        let (tx, rx) = mpsc::channel::<ActorMessage>();
        let started = Arc::new(Mutex::new(HashSet::new()));
        let started_for_thread = started.clone();
        std::thread::Builder::new()
            .name("transcription_actor".to_string())
            .spawn(move || {
                let asr = AsrService::new();
                let mut session: Option<ActorSession> = None;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        ActorMessage::Start { task_id, config } => {
                            if session.is_some() {
                                send_failed(
                                    &mailbox,
                                    &task_id,
                                    "E_STREAMING_TRANSCRIBE_BUSY",
                                    "another streaming transcription is already running",
                                );
                                continue;
                            }
                            match ActorSession::start(task_id.clone(), config, &mailbox) {
                                Ok(next) => {
                                    started_for_thread.lock().unwrap().insert(task_id);
                                    session = Some(next);
                                }
                                Err(e) => {
                                    started_for_thread.lock().unwrap().remove(&task_id);
                                    send_failed(
                                        &mailbox,
                                        &task_id,
                                        "E_STREAMING_TRANSCRIBE_START",
                                        e.to_string(),
                                    );
                                }
                            }
                        }
                        ActorMessage::AudioChunk {
                            task_id,
                            sequence,
                            pcm,
                            is_last,
                        } => {
                            let Some(active) = session.as_mut() else {
                                send_failed(
                                    &mailbox,
                                    &task_id,
                                    "E_STREAMING_TRANSCRIBE_SESSION_MISSING",
                                    "streaming transcription session is missing",
                                );
                                continue;
                            };
                            if active.task_id != task_id {
                                send_failed(
                                    &mailbox,
                                    &task_id,
                                    "E_STREAMING_TRANSCRIBE_TASK_MISMATCH",
                                    "streaming transcription task mismatch",
                                );
                                continue;
                            }
                            if let Err(e) =
                                active.handle_chunk(sequence, pcm, is_last, &asr, &mailbox)
                            {
                                send_failed(
                                    &mailbox,
                                    &task_id,
                                    "E_STREAMING_TRANSCRIBE_CHUNK",
                                    e.to_string(),
                                );
                            }
                        }
                        ActorMessage::Finish { task_id } => {
                            let Some(mut active) = session.take() else {
                                send_failed(
                                    &mailbox,
                                    &task_id,
                                    "E_STREAMING_TRANSCRIBE_SESSION_MISSING",
                                    "streaming transcription session is missing",
                                );
                                continue;
                            };
                            if active.task_id != task_id {
                                send_failed(
                                    &mailbox,
                                    &task_id,
                                    "E_STREAMING_TRANSCRIBE_TASK_MISMATCH",
                                    "streaming transcription task mismatch",
                                );
                                session = Some(active);
                                continue;
                            }
                            match active.finish(&mailbox) {
                                Ok(()) => {
                                    started_for_thread.lock().unwrap().remove(&task_id);
                                }
                                Err(e) => {
                                    started_for_thread.lock().unwrap().remove(&task_id);
                                    send_failed(
                                        &mailbox,
                                        &task_id,
                                        "E_STREAMING_TRANSCRIBE_FINISH",
                                        e.to_string(),
                                    );
                                }
                            }
                        }
                        ActorMessage::Cancel { task_id } => {
                            if let Some(mut active) = session.take() {
                                if active.task_id == task_id {
                                    active.cancel();
                                    mailbox.send(UiEvent::stage(
                                        &task_id,
                                        "Transcribe",
                                        UiEventStatus::Cancelled,
                                        "cancelled",
                                    ));
                                    mailbox.send(UiEvent::state_cancelled(&task_id, "Transcribe"));
                                } else {
                                    session = Some(active);
                                }
                            }
                            started_for_thread.lock().unwrap().remove(&task_id);
                        }
                    }
                }
            })
            .expect("failed to start transcription actor");
        Self { tx, started }
    }

    pub fn session_config_for_current_settings(&self) -> Result<StreamingSessionConfig> {
        let dir = data_dir::data_dir()?;
        let s = settings::load_settings_strict(&dir)?;
        let provider = StreamingProviderKind::from_settings(&s);
        let chunk_ms = match provider {
            StreamingProviderKind::Doubao => DOUBAO_CHUNK_MS,
            StreamingProviderKind::Local | StreamingProviderKind::Remote => LOCAL_CHUNK_MS,
        };
        Ok(StreamingSessionConfig {
            provider,
            chunk_ms,
            chunk_bytes: pcm_bytes_for_ms(chunk_ms),
        })
    }

    pub fn start_session(&self, task_id: &str, config: StreamingSessionConfig) -> Result<()> {
        self.tx
            .send(ActorMessage::Start {
                task_id: task_id.to_string(),
                config,
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))
    }

    pub fn send_audio_chunk(
        &self,
        task_id: &str,
        sequence: u64,
        pcm: Vec<u8>,
        is_last: bool,
    ) -> Result<()> {
        self.tx
            .send(ActorMessage::AudioChunk {
                task_id: task_id.to_string(),
                sequence,
                pcm,
                is_last,
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))
    }

    pub fn finish_session(&self, task_id: &str) -> Result<()> {
        self.tx
            .send(ActorMessage::Finish {
                task_id: task_id.to_string(),
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))
    }

    pub fn cancel_session(&self, task_id: &str) -> Result<()> {
        self.tx
            .send(ActorMessage::Cancel {
                task_id: task_id.to_string(),
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))
    }

    pub fn is_session_started(&self, task_id: &str) -> bool {
        self.started.lock().unwrap().contains(task_id)
    }
}

struct ActorSession {
    task_id: String,
    config: StreamingSessionConfig,
    started_at: Instant,
    text: String,
    local_segments: Vec<PathBuf>,
    doubao: Option<DoubaoSessionHandle>,
}

impl ActorSession {
    fn start(
        task_id: String,
        config: StreamingSessionConfig,
        mailbox: &UiEventMailbox,
    ) -> Result<Self> {
        mailbox.send(UiEvent::stage(
            &task_id,
            "Transcribe",
            UiEventStatus::Started,
            format!("asr({})", config.provider.as_str()),
        ));
        let doubao = if config.provider == StreamingProviderKind::Doubao {
            Some(DoubaoSessionHandle::start(
                task_id.clone(),
                mailbox.clone(),
            )?)
        } else {
            None
        };
        Ok(Self {
            task_id,
            config,
            started_at: Instant::now(),
            text: String::new(),
            local_segments: Vec::new(),
            doubao,
        })
    }

    fn handle_chunk(
        &mut self,
        sequence: u64,
        pcm: Vec<u8>,
        is_last: bool,
        asr: &AsrService,
        mailbox: &UiEventMailbox,
    ) -> Result<()> {
        if pcm.is_empty() && !is_last {
            return Ok(());
        }
        match self.config.provider {
            StreamingProviderKind::Doubao => {
                let Some(doubao) = self.doubao.as_ref() else {
                    return Err(anyhow!("doubao session missing"));
                };
                doubao.send_chunk(sequence, pcm, is_last)
            }
            StreamingProviderKind::Local => {
                if pcm.is_empty() {
                    return Ok(());
                }
                let text = transcribe_local_chunk(&self.task_id, sequence, pcm, asr)?;
                if !text.trim().is_empty() {
                    self.text.push_str(text.trim());
                    mailbox.send(UiEvent::partial(
                        &self.task_id,
                        text.trim(),
                        self.text.as_str(),
                        sequence,
                    ));
                }
                Ok(())
            }
            StreamingProviderKind::Remote => {
                Err(anyhow!("E_REMOTE_STREAMING_UNSUPPORTED: remote HTTP ASR does not support streaming actor mode"))
            }
        }
    }

    fn finish(&mut self, mailbox: &UiEventMailbox) -> Result<()> {
        if let Some(doubao) = self.doubao.take() {
            let text = doubao.finish()?;
            self.text = text;
        }
        if self.text.trim().is_empty() {
            return Err(anyhow!("E_ASR_EMPTY_TEXT: empty transcription"));
        }
        let elapsed = self.started_at.elapsed().as_millis();
        let result = TranscriptionResult::new(
            &self.task_id,
            self.text.trim().to_string(),
            TranscriptionMetrics {
                rtf: 0.0,
                device_used: self.config.provider.as_str().to_string(),
                preprocess_ms: 0,
                asr_ms: elapsed,
            },
        );
        append_history(&result)?;
        mailbox.send(UiEvent::stage_with_elapsed(
            &self.task_id,
            "Transcribe",
            UiEventStatus::Completed,
            "ok",
            Some(elapsed),
            None,
        ));
        mailbox.send(UiEvent::state_completed(
            &self.task_id,
            "transcription.completed",
            "transcription completed",
            serde_json::to_value(&result).unwrap_or_default(),
        ));
        Ok(())
    }

    fn cancel(&mut self) {
        if let Some(doubao) = self.doubao.take() {
            doubao.cancel();
        }
        for path in self.local_segments.drain(..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

struct DoubaoSessionHandle {
    tx: tokio::sync::mpsc::UnboundedSender<DoubaoCommand>,
    join: Option<std::thread::JoinHandle<Result<String>>>,
}

enum DoubaoCommand {
    Chunk {
        sequence: u64,
        pcm: Vec<u8>,
        is_last: bool,
    },
    Finish,
    Cancel,
}

impl DoubaoSessionHandle {
    fn start(task_id: String, mailbox: UiEventMailbox) -> Result<Self> {
        let creds = doubao_asr::load_credentials()?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let join = std::thread::Builder::new()
            .name("doubao_asr_session".to_string())
            .spawn(move || run_doubao_session(task_id, mailbox, creds, rx))
            .map_err(|e| anyhow!("spawn doubao session failed: {e}"))?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    fn send_chunk(&self, sequence: u64, pcm: Vec<u8>, is_last: bool) -> Result<()> {
        self.tx
            .send(DoubaoCommand::Chunk {
                sequence,
                pcm,
                is_last,
            })
            .map_err(|e| anyhow!("send doubao chunk failed: {e}"))
    }

    fn finish(mut self) -> Result<String> {
        let _ = self.tx.send(DoubaoCommand::Finish);
        match self.join.take() {
            Some(join) => join
                .join()
                .map_err(|_| anyhow!("doubao session thread panicked"))?,
            None => Err(anyhow!("doubao session join missing")),
        }
    }

    fn cancel(mut self) {
        let _ = self.tx.send(DoubaoCommand::Cancel);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run_doubao_session(
    task_id: String,
    mailbox: UiEventMailbox,
    creds: doubao_asr::DoubaoCredentials,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<DoubaoCommand>,
) -> Result<String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build doubao runtime failed")?;
    rt.block_on(async move {
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

        let (ws, resp) = tokio_tungstenite::connect_async(req)
            .await
            .context("connect doubao websocket failed")?;
        let logid = resp
            .headers()
            .get("X-Tt-Logid")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let (mut write, mut read) = ws.split();
        write
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                build_full_client_request_frame()?,
            ))
            .await
            .context("send doubao init frame failed")?;

        let mut full_text = String::new();
        let mut last_text = String::new();
        let mut finishing = false;
        loop {
            tokio::select! {
                cmd = recv_doubao_command(&mut rx) => {
                    match cmd? {
                        DoubaoCommand::Chunk { sequence, pcm, is_last } => {
                            if !pcm.is_empty() {
                                write
                                    .send(tokio_tungstenite::tungstenite::Message::Binary(
                                        build_audio_frame(sequence, &pcm, is_last)?,
                                    ))
                                    .await
                                    .context("send doubao audio frame failed")?;
                            }
                            if is_last {
                                write.flush().await.ok();
                            }
                        }
                        DoubaoCommand::Finish => {
                            finishing = true;
                            write.close().await.ok();
                        }
                        DoubaoCommand::Cancel => {
                            write.close().await.ok();
                            return Err(anyhow!("cancelled"));
                        }
                    }
                }
                msg = read.next() => {
                    let Some(msg) = msg else { break; };
                    let msg = msg.context("read doubao websocket message failed")?;
                    if !msg.is_binary() {
                        continue;
                    }
                    let payload = parse_server_payload(&msg.into_data())?;
                    if let Some(text) = extract_text(&payload.value) {
                        let delta = text_delta(&last_text, &text);
                        last_text = text.clone();
                        if !delta.trim().is_empty() {
                            full_text.push_str(delta.trim());
                            mailbox.send(UiEvent::partial(
                                &task_id,
                                delta.trim(),
                                full_text.as_str(),
                                0,
                            ));
                        }
                    }
                    if payload.is_last {
                        break;
                    }
                }
            }
            if finishing && last_text.trim().is_empty() {
                continue;
            }
        }
        if full_text.trim().is_empty() && !last_text.trim().is_empty() {
            full_text = last_text;
        }
        if let Some(logid) = logid {
            obs::event(
                &data_dir::data_dir()?,
                Some(&task_id),
                "Transcribe",
                "ASR.doubao_logid",
                "ok",
                Some(serde_json::json!({"x_tt_logid": logid})),
            );
        }
        Ok(full_text)
    })
}

async fn recv_doubao_command(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<DoubaoCommand>,
) -> Result<DoubaoCommand> {
    rx.recv()
        .await
        .ok_or_else(|| anyhow!("doubao command channel closed"))
}

fn build_full_client_request_frame() -> Result<Vec<u8>> {
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

fn build_audio_frame(sequence: u64, pcm: &[u8], is_last: bool) -> Result<Vec<u8>> {
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

struct DoubaoServerPayload {
    value: Value,
    is_last: bool,
}

fn parse_server_payload(frame: &[u8]) -> Result<DoubaoServerPayload> {
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

fn extract_text(payload: &Value) -> Option<String> {
    payload
        .get("result")
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

fn text_delta(previous: &str, current: &str) -> String {
    if current.starts_with(previous) {
        return current[previous.len()..].to_string();
    }
    current.to_string()
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

fn transcribe_local_chunk(
    task_id: &str,
    sequence: u64,
    pcm: Vec<u8>,
    asr: &AsrService,
) -> Result<String> {
    let dir = data_dir::data_dir()?;
    let path = write_pcm_wav(task_id, sequence, &pcm)?;
    let token = CancellationToken::new();
    let pid_slot = Arc::new(Mutex::new(None));
    let result = asr.transcribe(&dir, task_id, &path, "Chinese", &token, &pid_slot);
    let _ = std::fs::remove_file(&path);
    let (resp, _) = result?;
    if !resp.ok {
        let code = resp
            .error
            .as_ref()
            .map(|e| e.code.as_str())
            .unwrap_or("E_ASR_FAILED");
        let message = resp
            .error
            .as_ref()
            .map(|e| e.message.as_str())
            .unwrap_or("asr failed");
        return Err(anyhow!("{code}: {message}"));
    }
    Ok(resp.text.unwrap_or_default())
}

fn write_pcm_wav(task_id: &str, sequence: u64, pcm: &[u8]) -> Result<PathBuf> {
    let root = repo_root()?;
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp)?;
    let path = tmp.join(format!("{task_id}-stream-{sequence}.wav"));
    let mut bytes = Vec::with_capacity(44 + pcm.len());
    let data_len = pcm.len() as u32;
    let byte_rate = PCM_SAMPLE_RATE * u32::from(PCM_CHANNELS) * u32::from(PCM_BITS) / 8;
    let block_align = PCM_CHANNELS * PCM_BITS / 8;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&PCM_CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&PCM_SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&PCM_BITS.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.extend_from_slice(pcm);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn append_history(result: &TranscriptionResult) -> Result<()> {
    let dir = data_dir::data_dir()?;
    history::append(
        &dir.join("history.sqlite3"),
        &history::HistoryItem {
            task_id: result.transcript_id.clone(),
            created_at_ms: now_ms(),
            asr_text: result.asr_text.clone(),
            final_text: result.final_text.clone(),
            template_id: None,
            rtf: result.metrics.rtf,
            device_used: result.metrics.device_used.clone(),
            preprocess_ms: result.metrics.preprocess_ms as i64,
            asr_ms: result.metrics.asr_ms as i64,
        },
    )?;
    Ok(())
}

fn send_failed(mailbox: &UiEventMailbox, task_id: &str, code: &str, message: impl Into<String>) {
    let message = message.into();
    mailbox.send(UiEvent::stage_with_elapsed(
        task_id,
        "Transcribe",
        UiEventStatus::Failed,
        message.clone(),
        None,
        Some(code.to_string()),
    ));
    mailbox.send(UiEvent::state_failed(task_id, "Transcribe", code, message));
}

pub fn pcm_bytes_for_ms(ms: u64) -> usize {
    let bytes_per_second =
        PCM_SAMPLE_RATE as u64 * u64::from(PCM_CHANNELS) * u64::from(PCM_BITS / 8);
    ((bytes_per_second * ms) / 1000) as usize
}

fn repo_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_REPO_ROOT") {
        return Ok(PathBuf::from(p));
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = dir
        .ancestors()
        .nth(3)
        .ok_or_else(|| anyhow!("failed to locate repo root from CARGO_MANIFEST_DIR"))?;
    Ok(root.to_path_buf())
}

fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => dur.as_millis() as i64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_size_matches_pcm_duration() {
        assert_eq!(pcm_bytes_for_ms(200), 6_400);
        assert_eq!(pcm_bytes_for_ms(60_000), 1_920_000);
    }

    #[test]
    fn text_delta_returns_append_only_suffix() {
        assert_eq!(text_delta("你好", "你好世界"), "世界");
        assert_eq!(text_delta("旧", "新文本"), "新文本");
    }

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
