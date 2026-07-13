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
};

export type InsertResult = {
  copied: boolean;
  autoPasteAttempted: boolean;
  autoPasteOk: boolean;
  errorCode?: string | null;
  errorMessage?: string | null;
};

export type WorkflowMode = "ready" | "recording" | "processing" | "cancelling";

export type WorkflowStageKind =
  | "contextCapture"
  | "recordFinalize"
  | "preprocess"
  | "transcribe"
  | "rewrite"
  | "insertPrepare"
  | "finalize";

export type WorkflowStage = {
  kind: WorkflowStageKind;
  status: "pending" | "started" | "completed";
  elapsedMs?: number;
};

export type RunTimings = {
  totalMs: number;
  recordMs?: number;
  preprocessMs?: number;
  asrMs?: number;
  rewriteMs?: number;
};

export type RecoveredRunResult = {
  asrText: string;
  finalText: string;
  timings: RunTimings;
  metrics?: TranscriptionMetrics;
};

export type WorkflowError = {
  code: string;
  message: string;
};

export type CompletedRunResult = RecoveredRunResult & {
  insertResult: InsertResult;
  warning?: WorkflowError;
};

export type WorkflowOutcome =
  | { completed: CompletedRunResult }
  | { empty: { timings: RunTimings } }
  | {
    failed: {
      primaryError: WorkflowError;
      recoveredResult?: RecoveredRunResult;
      recoveryErrors: WorkflowError[];
      recordSaved: boolean;
      protocolContext?: {
        originalVariant: string;
        originalError?: WorkflowError;
      };
    };
  }
  | { cancelled: { recoveredResult?: RecoveredRunResult } };

export type ActiveRunProjection = {
  runId: string;
  stage?: WorkflowStage;
  result?: RecoveredRunResult;
};

export type LastRunProjection = {
  runId: string;
  outcome: WorkflowOutcome;
  stoppedCount: number;
  cleanupDiagnostic?: {
    elapsedMs: number;
    gracefulTimedOut: boolean;
    forceAttempted: boolean;
    forceSucceeded: boolean;
    detail?: string;
  };
  effects: {
    historyCommitCount: number;
    copyCount: number;
    pasteCount: number;
  };
  finalization: {
    commitCount: number;
  };
};

export type WorkflowView = {
  mode: WorkflowMode;
  revision: number;
  actionKey: string;
  activeRun: ActiveRunProjection | null;
  lastRun: LastRunProjection | null;
  primaryLabel: string;
  primaryDisabled: boolean;
  cancelEnabled: boolean;
};

export type WorkflowCommand = "primary" | "cancel";

export type WorkflowCommandRequest =
  | { command: "primary"; actionKey: string }
  | { command: "cancel"; targetRunId: string };

export type WorkflowCommandDisposition = "applied" | "noOp" | "cancelTooLate";

export type WorkflowCommandReply = {
  disposition: WorkflowCommandDisposition;
  view: WorkflowView;
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
  llm_prompt?: string | null;
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
  hotkey_primary?: string | null;
  hotkeys_show_overlay?: boolean | null;
  overlay_background_opacity?: number | null;
  overlay_font_size_px?: number | null;
  overlay_width_px?: number | null;
  overlay_height_px?: number | null;
  overlay_position_x?: number | null;
  overlay_position_y?: number | null;
};

export type OverlayConfig = {
  background_opacity: number;
  font_size_px: number;
  width_px: number;
  height_px: number;
  position_x?: number | null;
  position_y?: number | null;
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

export type ApiCheckResult = {
  ok: boolean;
  message: string;
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
