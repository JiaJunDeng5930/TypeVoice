import type { TaskEvent } from "../types";

export type DiagnosticView = {
  title: string;
  code: string;
  detail: string;
  actionHint: string;
};

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

export function extractErrorCode(raw: string): string | null {
  const m = raw.match(/\b(E_[A-Z0-9_]+|HTTP_\d{3})\b/);
  return m ? m[1] : null;
}

export function compactDetail(raw: string, maxChars = 220): string {
  const oneLine = raw.replace(/\s+/g, " ").trim();
  if (!oneLine) return "";
  if (oneLine.length <= maxChars) return oneLine;
  return `${oneLine.slice(0, maxChars)}...`;
}

function titleForCode(code: string, fallback: string): string {
  if (code.startsWith("E_SETTINGS_")) return "Settings need attention";
  if (code.startsWith("E_TOOLCHAIN_")) return "Local audio tools need repair";
  if (code.startsWith("E_CONTEXT_CAPTURE_")) return "App context is unavailable";
  if (code.startsWith("E_HOTKEY_")) return "Keyboard shortcut could not run";
  if (code === "E_RECORD_ALREADY_ACTIVE" || code === "E_TASK_ALREADY_ACTIVE") return "An action is already running";
  if (code === "E_RECORD_UNSUPPORTED") return "Recording is unavailable on this system";
  if (code.startsWith("E_RECORD_")) return "Recording could not start";
  if (code.startsWith("E_STREAMING_TRANSCRIBE_") || code.startsWith("E_DOUBAO_ASR_")) return "Speech recognition could not start";
  if (code.startsWith("E_REWRITE_") || code.startsWith("HTTP_")) return "Text improvement failed";
  if (code.startsWith("E_INSERT_") || code.startsWith("E_EXPORT_") || code.startsWith("E_OVERLAY_")) return "Text could not be pasted";
  if (code === "E_CMD_CANCEL") return "Cancel failed";
  return userTitleFromFallback(fallback);
}

function actionHintForCode(code: string): string {
  if (code.startsWith("E_TOOLCHAIN_")) return "Repair the local audio tools, then restart the app.";
  if (code.startsWith("E_RECORD_")) return "Check the selected microphone and try again.";
  if (code.startsWith("E_FFMPEG_")) return "Repair the local audio tools, then restart the app.";
  if (
    code.startsWith("E_ASR_") ||
    code.startsWith("E_STREAMING_TRANSCRIBE_") ||
    code.startsWith("E_DOUBAO_ASR_") ||
    code === "E_MODEL_LOAD_FAILED"
  ) {
    return "Check speech recognition settings and try again.";
  }
  if (code.startsWith("E_REWRITE_") || code.startsWith("HTTP_")) return "Check text improvement settings and try again.";
  if (code.startsWith("E_INSERT_") || code.startsWith("E_EXPORT_") || code.startsWith("E_OVERLAY_")) return "Select the target app and try again.";
  if (code === "E_HOTKEY_EVENT_INCOMPLETE") return "Restart the app, then try the shortcut again.";
  if (code === "E_TASK_ALREADY_ACTIVE" || code === "E_RECORD_ALREADY_ACTIVE") {
    return "Wait for the current action to finish.";
  }
  return "Check settings and try again.";
}

function userTitleFromFallback(fallback: string): string {
  const raw = fallback.toLowerCase();
  if (raw.includes("rewrite")) return "Text improvement failed";
  if (raw.includes("insert") || raw.includes("paste") || raw.includes("copy")) return "Text could not be pasted";
  if (raw.includes("transcribe") || raw.includes("workflow") || raw.includes("record")) return "Recording action failed";
  if (raw.includes("settings")) return "Settings need attention";
  return "Something went wrong";
}

export function buildDiagnostic(err: unknown, fallbackTitle: string): DiagnosticView {
  const raw = errorMessage(err);
  const code = extractErrorCode(raw) ?? "E_UNKNOWN";
  return {
    title: titleForCode(code, fallbackTitle),
    code,
    detail: compactDetail(raw || fallbackTitle),
    actionHint: actionHintForCode(code),
  };
}

export function buildTaskEventDiagnostic(ev: TaskEvent, fallbackTitle: string): DiagnosticView {
  const code = ev.error_code?.trim() || extractErrorCode(ev.message) || "E_UNKNOWN";
  return {
    title: ev.stage === "Rewrite" ? "Text improvement failed" : titleForCode(code, fallbackTitle),
    code,
    detail: compactDetail(ev.message || fallbackTitle),
    actionHint: actionHintForCode(code),
  };
}

export function toDiagnosticLine(diag: DiagnosticView): string {
  return `${diag.title}. ${diag.actionHint}`;
}

export function userMessageFromError(err: unknown, fallbackTitle = "Something went wrong"): string {
  return toDiagnosticLine(buildDiagnostic(err, fallbackTitle));
}

export function userMessageFromDiagnosticLine(raw: string): string {
  const compact = compactDetail(raw);
  if (!compact) return "";
  return extractErrorCode(compact) ? userMessageFromError(compact) : compact;
}

export function hotkeyCaptureHint(errCode?: string | null): string {
  if (!errCode) return "Keyboard shortcut could not run";
  if (errCode.includes("E_CONTEXT_SCREENSHOT_DISABLED")) return "Screenshot access is turned off";
  if (errCode.includes("E_TASK_ALREADY_ACTIVE")) return "An action is already running";
  if (errCode.includes("E_HOTKEY_EVENT_INCOMPLETE")) return "Restart the app, then try the shortcut again";
  if (errCode.includes("E_HOTKEY_TASK_OPEN")) return "Keyboard shortcut could not start recording";
  if (errCode.includes("E_HOTKEY_CAPTURE")) return "Current app context is unavailable";
  return "Keyboard shortcut could not run";
}
