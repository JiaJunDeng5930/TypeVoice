export type TaskEvent = {
  task_id: string;
  stage: string;
  status: "started" | "completed" | "failed" | "cancelled";
  message: string;
  elapsed_ms?: number | null;
  error_code?: string | null;
  diagnostic?: string | null;
  step_id?: string | null;
};

export type UiEvent = {
  kind: string;
  effect?: "displayOnly" | "stateChanging" | null;
  eventId?: string | null;
  sequence?: number | null;
  taskId?: string | null;
  stage?: string | null;
  status?: "started" | "completed" | "failed" | "cancelled" | "recording" | null;
  message: string;
  elapsedMs?: number | null;
  errorCode?: string | null;
  payload?: unknown;
  tsMs: number;
};

export type WorkflowApplyEventRequest = {
  eventId: string;
  kind: string;
  taskId?: string | null;
  status?: string | null;
  message: string;
  errorCode?: string | null;
  payload?: unknown;
};

export type WorkflowAsrCompletedRequest = {
  transcriptId: string;
  text: string;
  metrics: TranscriptionMetrics;
};

export type WorkflowTaskFailedRequest = {
  transcriptId: string;
  code: string;
  message: string;
};

export type WorkflowTextCommandRequest = {
  transcriptId: string;
  text: string;
};

export type RecordTranscribeStartResult = {
  sessionId: string;
};

export type TranscriptionMetrics = {
  rtf: number;
  deviceUsed: string;
  preprocessMs: number;
  asrMs: number;
};

export type TranscriptionResult = {
  transcriptId: string;
  asrText: string;
  finalText: string;
  metrics: TranscriptionMetrics;
  historyId: string;
};

export type RewriteResult = {
  transcriptId: string;
  finalText: string;
  rewriteMs: number;
  templateId?: string | null;
};

export type InsertResult = {
  copied: boolean;
  autoPasteAttempted: boolean;
  autoPasteOk: boolean;
  errorCode?: string | null;
  errorMessage?: string | null;
};

export type WorkflowCommand = "primary" | "rewriteLast" | "insertLast" | "copyLast" | "cancel";

export type WorkflowView = {
  phase: string;
  taskId?: string | null;
  recordingSessionId?: string | null;
  lastTranscriptId?: string | null;
  lastAsrText: string;
  lastText: string;
  lastCreatedAtMs?: number | null;
  diagnosticCode?: string | null;
  diagnosticLine: string;
  primaryLabel: string;
  primaryDisabled: boolean;
  canRewrite: boolean;
  canInsert: boolean;
  canCopy: boolean;
};

export type TaskDone = {
  task_id: string;
  asr_text: string;
  final_text: string;
  rtf: number;
  device_used: string;
  preprocess_ms: number;
  asr_ms: number;
  rewrite_ms?: number | null;
  rewrite_enabled: boolean;
  template_id?: string | null;
};

export type ExportTextResult = {
  copied: boolean;
  auto_paste_attempted: boolean;
  auto_paste_ok: boolean;
  error_code?: string | null;
  error_message?: string | null;
};

export type PromptTemplate = {
  id: string;
  name: string;
  system_prompt: string;
};

export type Settings = {
  asr_provider?: string | null;
  remote_asr_url?: string | null;
  remote_asr_model?: string | null;
  remote_asr_concurrency?: number | null;
  asr_preprocess_silence_trim_enabled?: boolean | null;
  asr_preprocess_silence_threshold_db?: number | null;
  asr_preprocess_silence_start_ms?: number | null;
  asr_preprocess_silence_end_ms?: number | null;
  llm_base_url?: string | null;
  llm_model?: string | null;
  llm_reasoning_effort?: string | null;
  record_input_spec?: string | null;
  record_input_strategy?: string | null;
  record_follow_default_role?: string | null;
  record_fixed_endpoint_id?: string | null;
  record_fixed_friendly_name?: string | null;
  record_last_working_endpoint_id?: string | null;
  record_last_working_friendly_name?: string | null;
  record_last_working_dshow_spec?: string | null;
  record_last_working_ts_ms?: number | null;
  rewrite_enabled?: boolean | null;
  rewrite_template_id?: string | null;
  rewrite_glossary?: string[] | null;
  auto_paste_enabled?: boolean | null;
  rewrite_include_glossary?: boolean | null;

  context_include_history?: boolean | null;
  context_history_n?: number | null;
  context_history_window_ms?: number | null;
  context_include_clipboard?: boolean | null;
  context_include_prev_window_screenshot?: boolean | null;
  context_include_prev_window_meta?: boolean | null;
  llm_supports_vision?: boolean | null;

  hotkeys_enabled?: boolean | null;
  hotkey_ptt?: string | null;
  hotkey_toggle?: string | null;
  hotkeys_show_overlay?: boolean | null;
};

export type AudioCaptureDevice = {
  endpoint_id: string;
  friendly_name: string;
  is_default_communications: boolean;
  is_default_console: boolean;
};

export type ApiKeyStatus = {
  configured: boolean;
  source: string;
  reason?: string | null;
};

export type RuntimeToolchainStatus = {
  ready: boolean;
  code?: string | null;
  message?: string | null;
  toolchain_dir?: string | null;
  platform: string;
  expected_version: string;
};

export type HistoryItem = {
  task_id: string;
  created_at_ms: number;
  asr_text: string;
  rewritten_text: string;
  inserted_text: string;
  final_text: string;
  template_id?: string | null;
  rtf: number;
  device_used: string;
  preprocess_ms: number;
  asr_ms: number;
};
