import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { copyText as browserCopyText } from "../lib/clipboard";

export type Unlisten = () => void;

export interface TauriGateway {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(eventName: string, handler: (payload: T) => void | Promise<void>): Promise<Unlisten>;
}

export interface TimerPort {
  setTimeout(callback: () => void, delayMs: number): number;
  clearTimeout(timerId: number): void;
}

export interface ClipboardPort {
  copyText(text: string): Promise<void>;
}

const previewNow = Date.now();
let previewSettings: Record<string, unknown> = {
  asr_provider: "doubao",
  remote_asr_url: "https://api.server/transcribe",
  remote_asr_model: "",
  remote_asr_concurrency: 4,
  asr_preprocess_silence_trim_enabled: false,
  asr_preprocess_silence_threshold_db: -50,
  asr_preprocess_silence_start_ms: 300,
  asr_preprocess_silence_end_ms: 300,
  llm_base_url: "",
  llm_model: "",
  llm_reasoning_effort: "default",
  llm_prompt: "",
  rewrite_enabled: true,
  rewrite_glossary: ["TypeVoice", "meeting note"],
  rewrite_include_glossary: true,
  auto_paste_enabled: true,
  record_input_strategy: "follow_default",
  record_follow_default_role: "communications",
  hotkeys_enabled: true,
  hotkey_ptt: "F9",
  hotkey_toggle: "F10",
  hotkeys_show_overlay: true,
  context_include_history: true,
  context_include_clipboard: true,
  context_include_prev_window_meta: true,
  context_include_prev_window_screenshot: false,
};

const previewWorkflow = {
  phase: "idle",
  taskId: null,
  recordingSessionId: null,
  lastTranscriptId: "preview-transcript",
  lastAsrText: "Send the revised note after the meeting, then attach the short summary below it.",
  lastText: "",
  lastCreatedAtMs: previewNow - 8 * 60 * 1000,
  diagnosticCode: null,
  diagnosticLine: "",
  primaryLabel: "START",
  primaryDisabled: false,
  canRewrite: false,
  canInsert: false,
  canCopy: false,
};

const previewHistory = [
  "Send the revised note after the meeting.",
  "Confirm the schedule for tomorrow morning.",
  "Move the follow-up task to next Tuesday.",
  "Summarize the discussion in three short bullets.",
  "Ask for the updated recording before noon.",
  "Draft the reply with a calmer tone.",
].map((text, index) => ({
  task_id: `preview-${index + 1}`,
  created_at_ms: previewNow - (index + 1) * 34 * 60 * 1000,
  asr_text: text,
  rewritten_text: "",
  inserted_text: "",
  final_text: text,
  template_id: null,
  rtf: 0.18,
  device_used: "Preview microphone",
  preprocess_ms: 12,
  asr_ms: 420,
}));

function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function compactDetail(raw: string, maxChars = 260): string {
  const oneLine = raw.replace(/\s+/g, " ").trim();
  if (!oneLine) return "";
  if (oneLine.length <= maxChars) return oneLine;
  return `${oneLine.slice(0, maxChars)}...`;
}

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "toString" in err) {
    try {
      return String(err);
    } catch {
      return "";
    }
  }
  return "";
}

function extractErrorCode(raw: string): string | null {
  const m = raw.match(/\b(E_[A-Z0-9_]+|HTTP_\d{3})\b/);
  return m ? m[1] : null;
}

const tauriGateway: TauriGateway = {
  async invoke<T>(command: string, args?: Record<string, unknown>) {
    try {
      return await tauriInvoke<T>(command, args);
    } catch (err) {
      if (command !== "ui_log_event") {
        const message = errorMessage(err);
        const code = extractErrorCode(message) || "E_INVOKE_FAILED";
        try {
          await tauriInvoke("ui_log_event", {
            req: {
              kind: "invoke_error",
              code,
              message: `${code}: ${compactDetail(message || "invoke failed")}`,
              detail: compactDetail(message || "invoke failed"),
              command,
              tsMs: Date.now(),
              extra: {
                hasArgs: !!args,
              },
            },
          });
        } catch {
          // ignore logger failure; preserve original invoke error
        }
      }
      throw err;
    }
  },
  async listen<T>(eventName: string, handler: (payload: T) => void | Promise<void>) {
    return tauriListen<T>(eventName, (event) => handler(event.payload));
  },
};

const browserPreviewGateway: TauriGateway = {
  async invoke<T>(command: string, args?: Record<string, unknown>) {
    switch (command) {
      case "ui_log_event":
      case "overlay_set_state":
      case "overlay_resize":
      case "set_llm_api_key":
      case "clear_llm_api_key":
      case "set_remote_asr_api_key":
      case "clear_remote_asr_api_key":
      case "set_doubao_asr_credentials":
      case "clear_doubao_asr_credentials":
        return undefined as T;
      case "check_llm_api_key":
        return { ok: true, message: "LLM API check passed." } as T;
      case "check_remote_asr_api_key":
        return { ok: true, message: "Remote ASR API check passed." } as T;
      case "check_doubao_asr_credentials":
        return { ok: true, message: "Doubao ASR API check passed." } as T;
      case "get_settings":
        return previewSettings as T;
      case "effective_settings_values":
        return {
          llm_base_url: previewSettings.llm_base_url,
          llm_model: previewSettings.llm_model,
        } as T;
      case "update_settings": {
        const patch = (args?.patch || {}) as Record<string, unknown>;
        previewSettings = { ...previewSettings, ...patch };
        return previewSettings as T;
      }
      case "workflow_snapshot":
      case "workflow_command":
      case "workflow_apply_event":
      case "workflow_report_asr_completed":
      case "workflow_report_asr_empty":
      case "workflow_report_asr_failed":
      case "workflow_report_rewrite_completed":
      case "workflow_report_rewrite_failed":
      case "workflow_report_insert_completed":
      case "workflow_report_insert_failed":
        return previewWorkflow as T;
      case "rewrite_text":
        return {
          transcriptId: String((args?.req as Record<string, unknown> | undefined)?.transcriptId || "preview-transcript"),
          finalText: String((args?.req as Record<string, unknown> | undefined)?.text || "").trim(),
          rewriteMs: 80,
        } as T;
      case "workflow_rewrite":
        return {
          transcriptId: "preview-transcript",
          finalText: String((args?.req as Record<string, unknown> | undefined)?.text || "").trim(),
          rewriteMs: 80,
        } as T;
      case "overlay_insert_text":
      case "workflow_insert":
        return {
          copied: true,
          autoPasteAttempted: true,
          autoPasteOk: true,
          errorCode: null,
          errorMessage: null,
        } as T;
      case "runtime_toolchain_status":
        return {
          ready: true,
          code: null,
          message: null,
          toolchain_dir: null,
          platform: "browser-preview",
          expected_version: "preview",
        } as T;
      case "history_list":
        return previewHistory as T;
      case "history_clear":
        return undefined as T;
      case "list_audio_capture_devices":
        return [
          {
            endpoint_id: "preview-mic",
            friendly_name: "Preview microphone",
            is_default_communications: true,
            is_default_console: true,
          },
        ] as T;
      case "check_hotkey_available":
        return { available: true, reason: null, reason_code: null } as T;
      case "llm_api_key_status":
      case "remote_asr_api_key_status":
      case "doubao_asr_credentials_status":
        return { configured: true, source: "browser-preview", reason: null } as T;
      default:
        throw new Error(`E_BROWSER_PREVIEW_UNSUPPORTED_COMMAND: ${command}`);
    }
  },
  async listen() {
    return () => {};
  },
};

export const defaultTauriGateway: TauriGateway = hasTauriRuntime()
  ? tauriGateway
  : browserPreviewGateway;

export const browserTimer: TimerPort = {
  setTimeout(callback: () => void, delayMs: number) {
    return window.setTimeout(callback, delayMs);
  },
  clearTimeout(timerId: number) {
    window.clearTimeout(timerId);
  },
};

export const browserClipboard: ClipboardPort = {
  async copyText(text: string) {
    await browserCopyText(text);
  },
};
