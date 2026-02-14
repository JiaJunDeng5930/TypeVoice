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
  asr_model?: string | null;
  asr_preprocess_silence_trim_enabled?: boolean | null;
  asr_preprocess_silence_threshold_db?: number | null;
  asr_preprocess_silence_start_ms?: number | null;
  asr_preprocess_silence_end_ms?: number | null;
  llm_base_url?: string | null;
  llm_model?: string | null;
  llm_reasoning_effort?: string | null;
  record_input_spec?: string | null;
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

export type ModelStatus = {
  model_dir: string;
  ok: boolean;
  reason?: string | null;
  model_version?: string | null;
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

export type RuntimePythonStatus = {
  ready: boolean;
  code?: string | null;
  message?: string | null;
  python_path?: string | null;
  python_version?: string | null;
};

export type HistoryItem = {
  task_id: string;
  created_at_ms: number;
  asr_text: string;
  final_text: string;
  template_id?: string | null;
  rtf: number;
  device_used: string;
  preprocess_ms: number;
  asr_ms: number;
};
