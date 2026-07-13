use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::{
    data_dir, doubao_asr, obs,
    pcm::{pcm_bytes_for_ms, pcm_peak_abs},
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
        ack: mpsc::Sender<FinishAck>,
    },
    Cancel {
        task_id: String,
    },
    Shutdown {
        ack: mpsc::Sender<()>,
    },
}

type StartAck = std::result::Result<(), String>;
type FinishAck = std::result::Result<TranscriptionResult, String>;

pub struct PendingSessionStart {
    ack: mpsc::Receiver<StartAck>,
}

impl PendingSessionStart {
    pub fn wait(self) -> Result<()> {
        match self.ack.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(anyhow!(message)),
            Err(e) => Err(anyhow!("E_STREAMING_ACTOR_ACK: {e}")),
        }
    }

    pub fn wait_timeout(self, timeout: Duration) -> Result<()> {
        match self.ack.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(anyhow!(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(anyhow!(
                "E_STREAMING_ACTOR_ACK_TIMEOUT: Start acknowledgement timed out"
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(anyhow!(
                "E_STREAMING_ACTOR_ACK: Start acknowledgement channel closed"
            )),
        }
    }
}

#[derive(Clone)]
pub struct TranscriptionActor {
    tx: mpsc::Sender<ActorMessage>,
    started: Arc<Mutex<HashSet<String>>>,
    cancel_tokens: Arc<Mutex<HashMap<String, CancellationToken>>>,
    join: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl TranscriptionActor {
    pub fn new(mailbox: UiEventMailbox) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<ActorMessage>();
        let started = Arc::new(Mutex::new(HashSet::new()));
        let started_for_thread = started.clone();
        let cancel_tokens = Arc::new(Mutex::new(HashMap::new()));
        let cancel_tokens_for_thread = cancel_tokens.clone();
        let join = std::thread::Builder::new()
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
                                cancel_tokens_for_thread.lock().unwrap().remove(&stale_task_id);
                            }
                            match ActorSession::start(task_id.clone(), config, &mailbox) {
                                Ok(next) => {
                                    if let Some(token) = next.cancel_token() {
                                        cancel_tokens_for_thread
                                            .lock()
                                            .unwrap()
                                            .insert(task_id.clone(), token);
                                    }
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
                        ActorMessage::Finish { task_id, ack } => {
                            let Some(mut active) = session.take() else {
                                started_for_thread.lock().unwrap().remove(&task_id);
                                let _ = ack.send(Err("E_STREAMING_SESSION_MISSING: no active session".to_string()));
                                continue;
                            };
                            if active.task_id != task_id {
                                started_for_thread.lock().unwrap().remove(&task_id);
                                session = Some(active);
                                let _ = ack.send(Err("E_STREAMING_SESSION_STALE: task id does not match active session".to_string()));
                                continue;
                            }
                            match active.finish(&mailbox) {
                                Ok(result) => {
                                    started_for_thread.lock().unwrap().remove(&task_id);
                                    cancel_tokens_for_thread.lock().unwrap().remove(&task_id);
                                    let _ = ack.send(Ok(result));
                                }
                                Err(e) => {
                                    started_for_thread.lock().unwrap().remove(&task_id);
                                    cancel_tokens_for_thread.lock().unwrap().remove(&task_id);
                                    let message = e.to_string();
                                    send_failed(
                                        &mailbox,
                                        &task_id,
                                        "E_STREAMING_TRANSCRIBE_FINISH",
                                        message.clone(),
                                    );
                                    let _ = ack.send(Err(message));
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
                                } else {
                                    session = Some(active);
                                }
                            }
                            started_for_thread.lock().unwrap().remove(&task_id);
                            cancel_tokens_for_thread.lock().unwrap().remove(&task_id);
                        }
                        ActorMessage::Shutdown { ack } => {
                            if let Some(mut active) = session.take() {
                                let task_id = active.task_id.clone();
                                active.cancel();
                                started_for_thread.lock().unwrap().remove(&task_id);
                                cancel_tokens_for_thread.lock().unwrap().remove(&task_id);
                            }
                            let _ = ack.send(());
                            break;
                        }
                    }
                }
            })
            .map_err(|error| anyhow!("E_STREAMING_ACTOR_SPAWN: {error}"))?;
        Ok(Self {
            tx,
            started,
            cancel_tokens,
            join: Arc::new(Mutex::new(Some(join))),
        })
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
        self.start_session_pending(task_id, config)?.wait()
    }

    pub fn start_session_pending(
        &self,
        task_id: &str,
        config: StreamingSessionConfig,
    ) -> Result<PendingSessionStart> {
        let (ack_tx, ack_rx) = mpsc::channel::<StartAck>();
        self.tx
            .send(ActorMessage::Start {
                task_id: task_id.to_string(),
                config,
                ack: ack_tx,
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))?;
        Ok(PendingSessionStart { ack: ack_rx })
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

    pub fn finish_session(&self, task_id: &str) -> Result<TranscriptionResult> {
        let (ack_tx, ack_rx) = mpsc::channel::<FinishAck>();
        self.tx
            .send(ActorMessage::Finish {
                task_id: task_id.to_string(),
                ack: ack_tx,
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))?;
        match ack_rx.recv() {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(message)) => Err(anyhow!(message)),
            Err(e) => Err(anyhow!("E_STREAMING_ACTOR_ACK: {e}")),
        }
    }

    pub fn cancel_session(&self, task_id: &str) -> Result<()> {
        if self.join.lock().unwrap().is_none() {
            return Ok(());
        }
        if let Some(token) = self.cancel_tokens.lock().unwrap().get(task_id).cloned() {
            token.cancel();
        }
        self.tx
            .send(ActorMessage::Cancel {
                task_id: task_id.to_string(),
            })
            .map_err(|e| anyhow!("E_STREAMING_ACTOR_SEND: {e}"))
    }

    pub fn is_session_started(&self, task_id: &str) -> bool {
        self.started.lock().unwrap().contains(task_id)
    }

    pub fn shutdown(&self) -> Result<()> {
        if self.join.lock().unwrap().is_none() {
            return Ok(());
        }
        for token in self.cancel_tokens.lock().unwrap().values() {
            token.cancel();
        }
        let (ack_tx, ack_rx) = mpsc::channel();
        let _ = self.tx.send(ActorMessage::Shutdown { ack: ack_tx });
        let _ = ack_rx.recv_timeout(Duration::from_millis(50));

        for _ in 0..10 {
            let finished = self
                .join
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished);
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut join = self.join.lock().unwrap();
        if join.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return Err(anyhow!(
                "E_STREAMING_ACTOR_SHUTDOWN_TIMEOUT: actor thread is still running"
            ));
        }
        if let Some(handle) = join.take() {
            let _ = handle.join();
        }
        Ok(())
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

    fn finish(&mut self, mailbox: &UiEventMailbox) -> Result<TranscriptionResult> {
        if let Some(doubao) = self.doubao.take() {
            let text = doubao.finish()?;
            self.text = text;
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
        if result.asr_text.is_empty() {
            mailbox.send(UiEvent::stage_with_elapsed(
                &self.task_id,
                "Transcribe",
                UiEventStatus::Completed,
                "empty",
                Some(elapsed),
                None,
            ));
            return Ok(result);
        }
        mailbox.send(UiEvent::stage_with_elapsed(
            &self.task_id,
            "Transcribe",
            UiEventStatus::Completed,
            "ok",
            Some(elapsed),
            None,
        ));
        Ok(result)
    }

    fn cancel(&mut self) {
        if let Some(doubao) = self.doubao.take() {
            doubao.cancel();
        }
    }

    fn cancel_token(&self) -> Option<CancellationToken> {
        self.doubao.as_ref().map(|handle| handle.cancel.clone())
    }
}

struct DoubaoSessionHandle {
    tx: tokio::sync::mpsc::UnboundedSender<DoubaoCommand>,
    cancel: CancellationToken,
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
}

impl DoubaoSessionHandle {
    fn start(task_id: String, mailbox: UiEventMailbox) -> Result<Self> {
        let creds = doubao_asr::load_credentials()?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let thread_cancel = cancel.clone();
        let join = std::thread::Builder::new()
            .name("doubao_asr_session".to_string())
            .spawn(move || run_doubao_session(task_id, mailbox, creds, rx, thread_cancel))
            .map_err(|e| anyhow!("spawn doubao session failed: {e}"))?;
        Ok(Self {
            tx,
            cancel,
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
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for DoubaoSessionHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
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
    cancel: CancellationToken,
) -> Result<String> {
    let task_id_for_trace = task_id.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build doubao runtime failed")?;
    let result = rt.block_on(async move {
        let req = doubao_asr::build_websocket_request(&creds)?;

        let connect = tokio_tungstenite::connect_async(req);
        tokio::pin!(connect);
        let (ws, resp) = tokio::select! {
            _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
            result = &mut connect => result.context("connect doubao websocket failed")?,
        };
        let logid = resp
            .headers()
            .get("X-Tt-Logid")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let (mut write, mut read) = ws.split();
        let init_frame = tokio_tungstenite::tungstenite::Message::Binary(
                doubao_asr::build_full_client_request_frame()?,
            );
        tokio::select! {
            _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
            result = write.send(init_frame) => {
                result.context("send doubao init frame failed")?;
            }
        }

        let mut final_text = String::new();
        let mut finishing = false;
        let mut finish_deadline: Option<tokio::time::Instant> = None;
        let mut stats = DoubaoSessionStats::default();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(anyhow!("cancelled"));
                }
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
                                let frame = tokio_tungstenite::tungstenite::Message::Binary(
                                    doubao_asr::build_audio_frame(sequence, &pcm, is_last)?,
                                );
                                tokio::select! {
                                    _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
                                    result = write.send(frame) => {
                                        result.context("send doubao audio frame failed")?;
                                    }
                                }
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
                                tokio::select! {
                                    _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
                                    _ = write.flush() => {}
                                }
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
                            tokio::select! {
                                _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
                                _ = write.flush() => {}
                            }
                        }
                    }
                }
                msg = read.next() => {
                    let Some(msg) = msg else {
                        if let Ok(dir) = data_dir::data_dir() {
                            let ctx = Some(serde_json::json!({
                                "finishing": finishing,
                                "final_text_chars": final_text.chars().count(),
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
                                    obs::ErrorEvent {
                                        task_id: Some(&task_id),
                                        stage: "Transcribe",
                                        step_id: "ASR.doubao_ws_closed",
                                        kind: "asr",
                                        code: "E_DOUBAO_ASR_WS_CLOSED",
                                        ctx,
                                    },
                                    "doubao websocket closed before finish",
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
                        if let Some(display_text) = replace_doubao_text(&mut final_text, text) {
                            mailbox.send(UiEvent::partial(
                                &task_id,
                                display_text.as_str(),
                                display_text.as_str(),
                                0,
                            ));
                        }
                    }
                    if payload.is_last {
                        break;
                    }
                }
            }
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
                    "final_text_chars": final_text.chars().count(),
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
        Ok(final_text)
    });
    if let Err(err) = &result {
        if let Ok(dir) = data_dir::data_dir() {
            obs::event_err_anyhow(
                &dir,
                obs::ErrorEvent {
                    task_id: Some(&task_id_for_trace),
                    stage: "Transcribe",
                    step_id: "ASR.doubao_session_failed",
                    kind: "asr",
                    code: "E_DOUBAO_ASR_SESSION_FAILED",
                    ctx: Some(serde_json::json!({"provider": "doubao"})),
                },
                err,
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

fn should_send_doubao_audio_frame(pcm: &[u8], is_last: bool) -> bool {
    !pcm.is_empty() || is_last
}

fn replace_doubao_text(current: &mut String, text: String) -> Option<String> {
    if text.trim().is_empty() || *current == text {
        return None;
    }
    *current = text;
    Some(current.clone())
}

fn send_failed(mailbox: &UiEventMailbox, task_id: &str, code: &str, message: impl Into<String>) {
    let message = message.into();
    if let Ok(dir) = data_dir::data_dir() {
        obs::event_err(
            &dir,
            obs::ErrorEvent {
                task_id: Some(task_id),
                stage: "Transcribe",
                step_id: "ASR.streaming_failed",
                kind: "asr",
                code,
                ctx: None,
            },
            &message,
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
    fn final_empty_doubao_chunk_is_sent() {
        assert!(should_send_doubao_audio_frame(&[], true));
        assert!(should_send_doubao_audio_frame(&[1, 2], false));
        assert!(!should_send_doubao_audio_frame(&[], false));
    }

    #[test]
    fn replace_doubao_text_uses_latest_complete_text() {
        let mut current = "旧文本".to_string();

        let next =
            replace_doubao_text(&mut current, "新文本".to_string()).expect("new text is published");

        assert_eq!(next, "新文本");
        assert_eq!(current, "新文本");
        assert_eq!(
            replace_doubao_text(&mut current, "新文本".to_string()),
            None
        );
    }

    #[test]
    fn second_start_replaces_existing_streaming_session() {
        let (mailbox, rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox).expect("actor");

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
        let error = actor
            .finish_session("task-1")
            .expect_err("old finish must be rejected as stale");
        assert!(error.to_string().contains("E_STREAMING_SESSION_STALE"));
        std::thread::sleep(Duration::from_millis(50));

        assert!(actor.is_session_started("task-2"));
        assert_no_failed_events(&rx);
    }

    #[test]
    fn start_session_returns_after_actor_marks_session_started() {
        let (mailbox, _rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox).expect("actor");

        actor
            .start_session("task-1", remote_streaming_config())
            .expect("start succeeds");

        assert!(actor.is_session_started("task-1"));
    }

    #[test]
    fn cancellation_is_idempotent_after_actor_shutdown() {
        let (mailbox, _rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox).expect("actor");
        actor
            .start_session("task-1", remote_streaming_config())
            .expect("start succeeds");

        actor.cancel_session("task-1").expect("cancel succeeds");
        actor.shutdown().expect("shutdown succeeds");

        actor
            .cancel_session("task-1")
            .expect("repeated cancel is a no-op");
        actor.shutdown().expect("repeated shutdown is a no-op");
    }

    #[test]
    fn missing_streaming_session_finish_is_stale() {
        let (mailbox, rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox).expect("actor");

        let error = actor
            .finish_session("task-1")
            .expect_err("missing finish must be rejected");
        assert!(error.to_string().contains("E_STREAMING_SESSION_MISSING"));
        std::thread::sleep(Duration::from_millis(50));

        assert_no_failed_events(&rx);
    }

    #[test]
    fn empty_streaming_finish_sends_stage_completion_without_failure() {
        let (mailbox, rx) = UiEventMailbox::for_test();
        let actor = TranscriptionActor::new(mailbox).expect("actor");

        actor
            .start_session("task-1", remote_streaming_config())
            .expect("start succeeds");
        actor.finish_session("task-1").expect("finish sends");
        std::thread::sleep(Duration::from_millis(50));

        let events: Vec<UiEvent> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|event| event.kind == "transcription.stage"
                    && event.status.as_deref() == Some("completed")
                    && event.message == "empty"),
            "expected empty stage completion: {events:?}"
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
