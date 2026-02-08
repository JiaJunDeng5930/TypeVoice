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
};

export type ModelStatus = {
  model_dir: string;
  ok: boolean;
  reason?: string | null;
};

export type ApiKeyStatus = {
  configured: boolean;
  source: string;
  reason?: string | null;
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

