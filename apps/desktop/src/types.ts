export type TaskEvent = {
  task_id: string;
  stage: string;
  status: "started" | "completed" | "failed" | "cancelled";
  message: string;
  elapsed_ms?: number | null;
  error_code?: string | null;
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

export type PromptTemplate = {
  id: string;
  name: string;
  system_prompt: string;
};

export type Settings = {
  asr_model?: string | null;
  llm_base_url?: string | null;
  llm_model?: string | null;
  llm_reasoning_effort?: string | null;
  rewrite_enabled?: boolean | null;
  rewrite_template_id?: string | null;
  rewrite_glossary?: string[] | null;

  context_include_history?: boolean | null;
  context_history_n?: number | null;
  context_history_window_ms?: number | null;
  context_include_clipboard?: boolean | null;
  context_include_prev_window_screenshot?: boolean | null;
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
