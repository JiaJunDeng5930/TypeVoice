use std::{
    collections::HashSet,
    sync::{mpsc, Arc, Mutex},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};

use crate::{
    data_dir, doubao_asr, obs,
    settings::{self, Settings},
    transcription::{TranscriptionMetrics, TranscriptionResult},
    ui_events::{UiEvent, UiEventMailbox, UiEventStatus},
};

const REMOTE_CHUNK_MS: u64 = 60_000;
const DOUBAO_CHUNK_MS: u64 = 200;
const DOUBAO_FINISH_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingProviderKind {
    Remote,
    Doubao,
}

impl StreamingProviderKind {
    fn from_settings(s: &Settings) -> Self {
        match settings::resolve_asr_provider(s).as_str() {
            "remote" => Self::Remote,
            _ => Self::Doubao,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
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
        ack: mpsc::Sender<StartAck>,
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

type StartAck = std::result::Result<(), String>;

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
                let mut session: Option<ActorSession> = None;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        ActorMessage::Start {
                            task_id,
                            config,
                            ack,
                        } => {
                            if let Some(mut active) = session.take() {
                                let stale_task_id = active.task_id.clone();
                                active.cancel();
                                started_for_thread.lock().unwrap().remove(&stale_task_id);
                            }
                            match ActorSession::start(task_id.clone(), config, &mailbox) {
                                Ok(next) => {
                                    started_for_thread.lock().unwrap().insert(task_id.clone());
                                    session = Some(next);
                                    let _ = ack.send(Ok(()));
                                }
                                Err(e) => {
                                    started_for_thread.lock().unwrap().remove(&task_id);
                                    let _ = ack.send(Err(e.to_string()));
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
                                started_for_thread.lock().unwrap().remove(&task_id);
                                continue;
                            };
                            if active.task_id != task_id {
                                started_for_thread.lock().unwrap().remove(&task_id);
                                continue;
                            }
                            if let Err(e) = active.handle_chunk(sequence, pcm, is_last) {
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
                                started_for_thread.lock().unwrap().remove(&task_id);
                                continue;
                            };
                            if active.task_id != task_id {
                                started_for_thread.lock().unwrap().remove(&task_id);
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
            StreamingProviderKind::Remote => REMOTE_CHUNK_MS,
        };
        Ok(StreamingSessionConfig {
            provider,
            chunk_ms,
            chunk_bytes: pcm_bytes_for_ms(chunk_ms),
        })
    }

    pub fn start_session(&self, task_id: &str, config: StreamingSessionConfig) -> Result<()> {
        let (ack_tx, ack_rx) = mpsc::channel::<StartAck>();
        self.tx
            .send(ActorMessage::Start {
                task_id: task_id.to_string(),
                config,
                ack: ack_tx,
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))?;
        match ack_rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(anyhow!(message)),
            Err(e) => Err(anyhow!("E_STREAMING_ACTOR_ACK: {e}")),
        }
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
    doubao: Option<DoubaoSessionHandle>,
}

impl ActorSession {
    fn start(
        task_id: String,
        config: StreamingSessionConfig,
        mailbox: &UiEventMailbox,
    ) -> Result<Self> {
        let doubao = if config.provider == StreamingProviderKind::Doubao {
            Some(DoubaoSessionHandle::start(
                task_id.clone(),
                mailbox.clone(),
            )?)
        } else {
            None
        };
        mailbox.send(UiEvent::stage(
            &task_id,
            "Transcribe",
            UiEventStatus::Started,
            format!("asr({})", config.provider.as_str()),
        ));
        Ok(Self {
            task_id,
            config,
            started_at: Instant::now(),
            text: String::new(),
            doubao,
        })
    }

    fn handle_chunk(&mut self, sequence: u64, pcm: Vec<u8>, is_last: bool) -> Result<()> {
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
            mailbox.send(UiEvent::stage_with_elapsed(
                &self.task_id,
                "Transcribe",
                UiEventStatus::Completed,
                "empty",
                Some(self.started_at.elapsed().as_millis()),
                None,
            ));
            mailbox.send(UiEvent::transcription_empty(&self.task_id));
            return Ok(());
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
        mailbox.send(UiEvent::stage_with_elapsed(
            &self.task_id,
            "Transcribe",
            UiEventStatus::Completed,
            "ok",
            Some(elapsed),
            None,
        ));
        mailbox.send(UiEvent::completed(
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
    }
}

struct DoubaoSessionHandle {
    tx: tokio::sync::mpsc::UnboundedSender<DoubaoCommand>,
    join: Option<std::thread::JoinHandle<Result<String>>>,
}

#[derive(Default)]
struct DoubaoSessionStats {
    sent_frames: usize,
    sent_empty_frames: usize,
    sent_audio_bytes: usize,
    non_silent_frames: usize,
    first_sequence: Option<u64>,
    last_sequence: Option<u64>,
    last_frame_marked: bool,
    binary_responses: usize,
    text_responses: usize,
    non_binary_responses: usize,
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
    let task_id_for_trace = task_id.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build doubao runtime failed")?;
    let result = rt.block_on(async move {
        let req = doubao_asr::build_websocket_request(&creds)?;

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
                doubao_asr::build_full_client_request_frame()?,
            ))
            .await
            .context("send doubao init frame failed")?;

        let mut full_text = String::new();
        let mut last_text = String::new();
        let mut finishing = false;
        let mut finish_deadline: Option<tokio::time::Instant> = None;
        let mut stats = DoubaoSessionStats::default();
        loop {
            tokio::select! {
                _ = async {
                    if let Some(deadline) = finish_deadline {
                        tokio::time::sleep_until(deadline).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }, if finish_deadline.is_some() => {
                    return Err(anyhow!(
                        "E_DOUBAO_ASR_FINISH_TIMEOUT: timed out waiting for final transcription"
                    ));
                }
                cmd = recv_doubao_command(&mut rx) => {
                    match cmd? {
                        DoubaoCommand::Chunk { sequence, pcm, is_last } => {
                            if should_send_doubao_audio_frame(&pcm, is_last) {
                                write
                                    .send(tokio_tungstenite::tungstenite::Message::Binary(
                                        doubao_asr::build_audio_frame(sequence, &pcm, is_last)?,
                                    ))
                                    .await
                                    .context("send doubao audio frame failed")?;
                                stats.sent_frames += 1;
                                stats.sent_audio_bytes += pcm.len();
                                if pcm.is_empty() {
                                    stats.sent_empty_frames += 1;
                                } else {
                                    stats.non_silent_frames += usize::from(pcm_peak_abs(&pcm) > 0);
                                }
                                stats.first_sequence.get_or_insert(sequence);
                                stats.last_sequence = Some(sequence);
                            } else {
                                stats.sent_empty_frames += 1;
                                stats.first_sequence.get_or_insert(sequence);
                                stats.last_sequence = Some(sequence);
                            }
                            if is_last {
                                stats.last_frame_marked = true;
                                write.flush().await.ok();
                                finishing = true;
                                finish_deadline = Some(
                                    tokio::time::Instant::now()
                                        + std::time::Duration::from_secs(DOUBAO_FINISH_TIMEOUT_SECS),
                                );
                            }
                        }
                        DoubaoCommand::Finish => {
                            finishing = true;
                            if finish_deadline.is_none() {
                                finish_deadline = Some(
                                    tokio::time::Instant::now()
                                        + std::time::Duration::from_secs(DOUBAO_FINISH_TIMEOUT_SECS),
                                );
                            }
                            write.flush().await.ok();
                        }
                        DoubaoCommand::Cancel => {
                            write.close().await.ok();
                            return Err(anyhow!("cancelled"));
                        }
                    }
                }
                msg = read.next() => {
                    let Some(msg) = msg else {
                        if let Ok(dir) = data_dir::data_dir() {
                            let ctx = Some(serde_json::json!({
                                "finishing": finishing,
                                "full_text_chars": full_text.chars().count(),
                                "last_text_chars": last_text.chars().count(),
                                "x_tt_logid": logid.as_deref(),
                                "sent_frames": stats.sent_frames,
                                "sent_empty_frames": stats.sent_empty_frames,
                                "sent_audio_bytes": stats.sent_audio_bytes,
                                "non_silent_frames": stats.non_silent_frames,
                                "first_sequence": stats.first_sequence,
                                "last_sequence": stats.last_sequence,
                                "last_frame_marked": stats.last_frame_marked,
                                "binary_responses": stats.binary_responses,
                                "text_responses": stats.text_responses,
                                "non_binary_responses": stats.non_binary_responses,
                            }));
                            if finishing {
                                obs::event(
                                    &dir,
                                    Some(&task_id),
                                    "Transcribe",
                                    "ASR.doubao_ws_closed",
                                    "ok",
                                    ctx,
                                );
                            } else {
                                obs::event_err(
                                    &dir,
                                    Some(&task_id),
                                    "Transcribe",
                                    "ASR.doubao_ws_closed",
                                    "asr",
                                    "E_DOUBAO_ASR_WS_CLOSED",
                                    "doubao websocket closed before finish",
                                    ctx,
                                );
                            }
                        }
                        break;
                    };
                    let msg = msg.context("read doubao websocket message failed")?;
                    if !msg.is_binary() {
                        stats.non_binary_responses += 1;
                        continue;
                    }
                    stats.binary_responses += 1;
                    let payload = doubao_asr::parse_server_payload(&msg.into_data())?;
                    let text = doubao_asr::extract_text(&payload.value);
                    let text_chars = text.as_ref().map(|v| v.chars().count()).unwrap_or(0);
                    if text.is_some() {
                        stats.text_responses += 1;
                    }
                    if let Ok(dir) = data_dir::data_dir() {
                        obs::event(
                            &dir,
                            Some(&task_id),
                            "Transcribe",
                            "ASR.doubao_response",
                            "ok",
                            Some(serde_json::json!({
                                "binary_response_index": stats.binary_responses,
                                "has_text": text.is_some(),
                                "text_chars": text_chars,
                                "is_last": payload.is_last,
                                "has_result": payload.value.get("result").is_some(),
                                "has_audio_info": payload.value.get("audio_info").is_some(),
                            })),
                        );
                    }
                    if let Some(text) = text {
                        if let Some(delta) =
                            append_doubao_text_delta(&mut full_text, &mut last_text, text)
                        {
                            mailbox.send(UiEvent::partial(&task_id, &delta, full_text.as_str(), 0));
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
            full_text = last_text.clone();
        }
        if let Ok(dir) = data_dir::data_dir() {
            obs::event(
                &dir,
                Some(&task_id),
                "Transcribe",
                "ASR.doubao_session_summary",
                "ok",
                Some(serde_json::json!({
                    "finishing": finishing,
                    "full_text_chars": full_text.chars().count(),
                    "last_text_chars": last_text.chars().count(),
                    "x_tt_logid": logid.as_deref(),
                    "sent_frames": stats.sent_frames,
                    "sent_empty_frames": stats.sent_empty_frames,
                    "sent_audio_bytes": stats.sent_audio_bytes,
                    "non_silent_frames": stats.non_silent_frames,
                    "first_sequence": stats.first_sequence,
                    "last_sequence": stats.last_sequence,
                    "last_frame_marked": stats.last_frame_marked,
                    "binary_responses": stats.binary_responses,
                    "text_responses": stats.text_responses,
                    "non_binary_responses": stats.non_binary_responses,
                })),
            );
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
    });
    if let Err(err) = &result {
        if let Ok(dir) = data_dir::data_dir() {
            obs::event_err_anyhow(
                &dir,
                Some(&task_id_for_trace),
                "Transcribe",
                "ASR.doubao_session_failed",
                "asr",
                "E_DOUBAO_ASR_SESSION_FAILED",
                err,
                Some(serde_json::json!({"provider": "doubao"})),
            );
        }
    }
    result
}

async fn recv_doubao_command(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<DoubaoCommand>,
) -> Result<DoubaoCommand> {
    rx.recv()
        .await
        .ok_or_else(|| anyhow!("doubao command channel closed"))
}

fn text_delta(previous: &str, current: &str) -> String {
    if current.starts_with(previous) {
        return current[previous.len()..].to_string();
    }
    current.to_string()
}

fn should_send_doubao_audio_frame(pcm: &[u8], is_last: bool) -> bool {
    !pcm.is_empty() || is_last
}

fn append_doubao_text_delta(
    full_text: &mut String,
    last_text: &mut String,
    text: String,
) -> Option<String> {
    let delta = text_delta(last_text, &text);
    *last_text = text;
    if delta.trim().is_empty() {
        return None;
    }
    full_text.push_str(&delta);
    Some(delta)
}

fn send_failed(mailbox: &UiEventMailbox, task_id: &str, code: &str, message: impl Into<String>) {
    let message = message.into();
    if let Ok(dir) = data_dir::data_dir() {
        obs::event_err(
            &dir,
            Some(task_id),
            "Transcribe",
            "ASR.streaming_failed",
            "asr",
            code,
            &message,
            None,
        );
    }
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

fn pcm_peak_abs(pcm: &[u8]) -> i32 {
    pcm.chunks_exact(2)
        .map(|bytes| i32::from(i16::from_le_bytes([bytes[0], bytes[1]])).abs())
        .max()
        .unwrap_or(0)
}

pub fn pcm_bytes_for_ms(ms: u64) -> usize {
    let bytes_per_second = doubao_asr::PCM_SAMPLE_RATE as u64
        * u64::from(doubao_asr::PCM_CHANNELS)
        * u64::from(doubao_asr::PCM_BITS / 8);
    ((bytes_per_second * ms) / 1000) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::ui_events::{UiEvent, UiEventMailbox};

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
    fn final_empty_doubao_chunk_is_sent() {
        assert!(should_send_doubao_audio_frame(&[], true));
        assert!(should_send_doubao_audio_frame(&[1, 2], false));
        assert!(!should_send_doubao_audio_frame(&[], false));
    }

    #[test]
    fn append_doubao_delta_preserves_leading_whitespace() {
        let mut full_text = "hello".to_string();
        let mut last_text = "hello".to_string();

        let delta =
            append_doubao_text_delta(&mut full_text, &mut last_text, "hello world".to_string())
                .expect("delta contains text");

        assert_eq!(delta, " world");
        assert_eq!(full_text, "hello world");
        assert_eq!(last_text, "hello world");
    }

    #[test]
    fn second_start_replaces_existing_streaming_session() {
        let (mailbox, rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox);

        actor
            .start_session("task-1", remote_streaming_config())
            .expect("first start sends");
        wait_until(|| actor.is_session_started("task-1"));

        actor
            .start_session("task-2", remote_streaming_config())
            .expect("second start sends");
        wait_until(|| actor.is_session_started("task-2") && !actor.is_session_started("task-1"));

        actor
            .send_audio_chunk("task-1", 1, vec![0, 0], false)
            .expect("old chunk sends");
        actor.finish_session("task-1").expect("old finish sends");
        std::thread::sleep(Duration::from_millis(50));

        assert!(actor.is_session_started("task-2"));
        assert_no_failed_events(&rx);
    }

    #[test]
    fn start_session_returns_after_actor_marks_session_started() {
        let (mailbox, _rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox);

        actor
            .start_session("task-1", remote_streaming_config())
            .expect("start succeeds");

        assert!(actor.is_session_started("task-1"));
    }

    #[test]
    fn missing_streaming_session_finish_is_stale() {
        let (mailbox, rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox);

        actor.finish_session("task-1").expect("finish sends");
        std::thread::sleep(Duration::from_millis(50));

        assert_no_failed_events(&rx);
    }

    #[test]
    fn empty_streaming_finish_sends_empty_event_without_failure() {
        let (mailbox, rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox);

        actor
            .start_session("task-1", remote_streaming_config())
            .expect("start succeeds");
        actor.finish_session("task-1").expect("finish sends");
        std::thread::sleep(Duration::from_millis(50));

        let events: Vec<UiEvent> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|event| event.kind == "transcription.empty"),
            "expected empty transcription event: {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|event| event.status.as_deref() != Some("failed")),
            "unexpected failed event: {events:?}"
        );
    }

    fn remote_streaming_config() -> StreamingSessionConfig {
        StreamingSessionConfig {
            provider: StreamingProviderKind::Remote,
            chunk_ms: 1,
            chunk_bytes: 2,
        }
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..100 {
            if condition() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("condition did not become true");
    }

    fn assert_no_failed_events(rx: &mpsc::Receiver<UiEvent>) {
        let events: Vec<UiEvent> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .all(|event| event.status.as_deref() != Some("failed")),
            "unexpected failed event: {events:?}"
        );
    }
}
