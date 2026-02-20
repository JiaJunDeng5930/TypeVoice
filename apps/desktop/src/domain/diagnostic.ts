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
  if (code.startsWith("E_SETTINGS_")) return "SETTINGS INVALID";
  if (code === "E_TOOLCHAIN_NOT_READY") return "TOOLCHAIN NOT READY";
  if (code === "E_TOOLCHAIN_CHECKSUM_MISMATCH") return "TOOLCHAIN CHECKSUM ERROR";
  if (code === "E_TOOLCHAIN_VERSION_MISMATCH") return "TOOLCHAIN VERSION ERROR";
  if (code === "E_PYTHON_NOT_READY") return "PYTHON NOT READY";
  if (code === "E_CONTEXT_CAPTURE_REQUIRED") return "CONTEXT CAPTURE REQUIRED";
  if (code === "E_CONTEXT_CAPTURE_NOT_FOUND") return "CONTEXT CAPTURE EXPIRED";
  if (code === "E_CONTEXT_CAPTURE_INVALID") return "CONTEXT CAPTURE INVALID";
  if (code === "E_RECORDING_SESSION_OPEN") return "RECORDING SESSION FAILED";
  if (code === "E_RECORD_ALREADY_ACTIVE") return "RECORDING BUSY";
  if (code === "E_TASK_ALREADY_ACTIVE") return "TASK BUSY";
  if (code === "E_RECORD_UNSUPPORTED") return "RECORDING UNSUPPORTED";
  if (code.startsWith("E_RECORD_")) return "RECORDING FAILED";
  if (code === "E_CMD_CANCEL") return "CANCEL FAILED";
  if (code.startsWith("HTTP_")) return "LLM REQUEST FAILED";
  return fallback;
}

function actionHintForCode(code: string): string {
  if (code.startsWith("E_TOOLCHAIN_")) return "RUN WINDOWS GATE TO REPAIR TOOLCHAIN";
  if (code === "E_PYTHON_NOT_READY") return "CHECK PYTHON ENV (.venv / TYPEVOICE_PYTHON)";
  if (code.startsWith("E_RECORD_")) return "CHECK MICROPHONE INPUT SPEC / DEVICE";
  if (code.startsWith("E_FFMPEG_")) return "CHECK FFMPEG TOOLCHAIN";
  if (code.startsWith("E_ASR_") || code === "E_MODEL_LOAD_FAILED") {
    return "CHECK ASR MODEL + CUDA RUNTIME";
  }
  if (code.startsWith("HTTP_")) return "CHECK LLM ENDPOINT / API KEY";
  if (code === "E_TASK_ALREADY_ACTIVE" || code === "E_RECORD_ALREADY_ACTIVE") {
    return "WAIT FOR CURRENT TASK OR RECORDING TO FINISH";
  }
  return "CHECK TRACE.JSONL WITH THIS ERROR CODE";
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
    title: ev.stage === "Rewrite" ? "REWRITE FAILED" : titleForCode(code, fallbackTitle),
    code,
    detail: compactDetail(ev.message || fallbackTitle),
    actionHint: actionHintForCode(code),
  };
}

export function toDiagnosticLine(diag: DiagnosticView): string {
  return `[${diag.code}] ${diag.detail} | ${diag.actionHint}`;
}

export function hotkeyCaptureHint(errCode?: string | null): string {
  if (!errCode) return "HOTKEY CAPTURE FAILED";
  if (errCode.includes("E_CONTEXT_SCREENSHOT_DISABLED")) return "SCREENSHOT DISABLED";
  if (errCode.includes("E_RECORDING_SESSION_OPEN")) return "HOTKEY SESSION OPEN FAILED";
  if (errCode.includes("E_HOTKEY_CAPTURE")) return "WINDOW CAPTURE FAILED";
  return "HOTKEY CAPTURE FAILED";
}
