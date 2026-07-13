import { userMessageFromDiagnosticLine } from "./diagnostic.ts";
import type {
  ActiveRunProjection,
  LastRunProjection,
  WorkflowCommandDisposition,
  WorkflowCommandReply,
  WorkflowMode,
  WorkflowView,
} from "../types";

export type WorkflowPayload = WorkflowView | WorkflowCommandReply;

export type WorkflowPresentationPhase =
  | "idle"
  | "recording"
  | "transcribing"
  | "transcribed"
  | "failed";

export type OverlayTone = "default" | "ok" | "danger";

export type OverlayViewState = {
  visible: boolean;
  status: string;
  detail: string | null;
  tone: OverlayTone;
};

export type WorkflowDiagnostic = {
  code: string | null;
  message: string | null;
};

export const EMPTY_WORKFLOW_VIEW: WorkflowView = {
  mode: "ready",
  revision: 0,
  actionKey: "Unavailable",
  activeRun: null,
  lastRun: null,
  primaryLabel: "LOADING",
  primaryDisabled: true,
  cancelEnabled: false,
};

export function workflowViewFromPayload(payload: unknown): WorkflowPayload | null {
  if (!isRecord(payload)) return null;
  if (hasOwn(payload, "disposition") || hasOwn(payload, "view")) {
    return commandReplyFromRecord(payload);
  }
  return workflowViewFromRecord(payload);
}

export function isWorkflowCommandReply(payload: WorkflowPayload): payload is WorkflowCommandReply {
  return "disposition" in payload;
}

export function workflowProjectionFromPayload(payload: unknown): WorkflowView | null {
  const parsed = workflowViewFromPayload(payload);
  if (!parsed || isWorkflowCommandReply(parsed)) return null;
  return parsed;
}

export function shouldAcceptWorkflowProjection(
  latestRevision: number | null,
  candidate: unknown,
): boolean {
  const revision = workflowProjectionRevision(candidate);
  if (revision === null) return false;
  if (latestRevision === null) return true;
  return Number.isSafeInteger(latestRevision) && latestRevision >= 0 && revision > latestRevision;
}

export function workflowProjectionRevision(candidate: unknown): number | null {
  if (!isRecord(candidate)) return null;
  if (hasOwn(candidate, "disposition") || hasOwn(candidate, "view")) {
    const reply = commandReplyFromRecord(candidate);
    return reply ? reply.view.revision : null;
  }
  return workflowViewFromRecord(candidate)?.revision ?? null;
}

export function workflowPresentationPhase(view: WorkflowView): WorkflowPresentationPhase {
  if (view.mode === "recording") return "recording";
  if (view.mode === "processing" || view.mode === "cancelling") return "transcribing";
  const outcome = workflowOutcomeKind(view);
  if (outcome === "failed") return "failed";
  if (outcome === "completed") return "transcribed";
  return "idle";
}

export function overlayViewFromWorkflow(view: WorkflowView): OverlayViewState {
  const diagnostic = workflowDiagnostic(view);
  const text = workflowDisplayText(view);
  const outcome = workflowOutcomeKind(view);
  const status = statusLabelFromWorkflow(view, outcome);
  return {
    visible: view.mode !== "ready" || Boolean(text) || outcome === "failed",
    status,
    detail: diagnostic.message
      ? userMessageFromDiagnosticLine(diagnostic.message)
      : null,
    tone: outcome === "failed" ? "danger" : outcome === "completed" ? "ok" : "default",
  };
}

export function workflowDisplayText(view: WorkflowView): string {
  const candidates = [
    valueAt(view.activeRun, "result", "finalText"),
    valueAt(view.activeRun, "result", "asrText"),
    valueAt(view.lastRun, "outcome", "completed", "finalText"),
    valueAt(view.lastRun, "outcome", "completed", "asrText"),
    valueAt(view.lastRun, "outcome", "failed", "recoveredResult", "finalText"),
    valueAt(view.lastRun, "outcome", "failed", "recoveredResult", "asrText"),
    valueAt(view.lastRun, "outcome", "cancelled", "recoveredResult", "finalText"),
    valueAt(view.lastRun, "outcome", "cancelled", "recoveredResult", "asrText"),
    valueAt(view.lastRun, "result", "finalText"),
    valueAt(view.lastRun, "result", "asrText"),
  ];
  for (const value of candidates) {
    if (typeof value === "string" && value.trim()) return value;
  }
  return "";
}

export function workflowDiagnostic(view: WorkflowView): WorkflowDiagnostic {
  const error = valueAt(view.lastRun, "outcome", "failed", "primaryError");
  if (!isRecord(error)) return { code: null, message: null };
  return {
    code: optionalString(error.code),
    message: optionalString(error.message) || optionalString(error.summary),
  };
}

export function primaryActionLabel(raw: string): string {
  const label = raw.trim().toUpperCase();
  if (label === "START") return "Start";
  if (label === "STOP") return "Stop";
  if (label === "CANCEL") return "Cancel";
  return raw.trim() || "Start";
}

function workflowViewFromRecord(raw: Record<string, unknown>): WorkflowView | null {
  if (!isRecordWithKeys(raw, [
    "mode",
    "revision",
    "actionKey",
    "activeRun",
    "lastRun",
    "primaryLabel",
    "primaryDisabled",
    "cancelEnabled",
  ], [])) return null;
  const mode = workflowMode(raw.mode);
  if (!mode || !isRevision(raw.revision)) return null;
  if (typeof raw.actionKey !== "string" || !raw.actionKey.trim()) return null;
  if (typeof raw.primaryLabel !== "string" || !raw.primaryLabel.trim()) return null;
  if (typeof raw.primaryDisabled !== "boolean" || typeof raw.cancelEnabled !== "boolean") {
    return null;
  }
  if (!hasOwn(raw, "activeRun") || !hasOwn(raw, "lastRun")) return null;

  const activeRun = nullableActiveRunProjection(raw.activeRun);
  const lastRun = nullableLastRunProjection(raw.lastRun);
  if (!activeRun.valid || !lastRun.valid) return null;
  if (mode === "ready" && activeRun.value !== null) return null;
  if (mode !== "ready" && activeRun.value === null) return null;

  return {
    mode,
    revision: raw.revision,
    actionKey: raw.actionKey,
    activeRun: activeRun.value,
    lastRun: lastRun.value,
    primaryLabel: raw.primaryLabel,
    primaryDisabled: raw.primaryDisabled,
    cancelEnabled: raw.cancelEnabled,
  };
}

function commandReplyFromRecord(raw: Record<string, unknown>): WorkflowCommandReply | null {
  if (!isRecordWithKeys(raw, ["disposition", "view"], [])) return null;
  const disposition = commandDisposition(raw.disposition);
  if (!disposition || !isRecord(raw.view)) return null;
  const view = workflowViewFromRecord(raw.view);
  return view ? { disposition, view } : null;
}

function workflowMode(value: unknown): WorkflowMode | null {
  return value === "ready"
    || value === "recording"
    || value === "processing"
    || value === "cancelling"
    ? value
    : null;
}

function commandDisposition(value: unknown): WorkflowCommandDisposition | null {
  return value === "applied" || value === "noOp" || value === "cancelTooLate"
    ? value
    : null;
}

function nullableActiveRunProjection(value: unknown): {
  valid: boolean;
  value: ActiveRunProjection | null;
} {
  if (value === null) return { valid: true, value: null };
  return isActiveRunProjection(value)
    ? { valid: true, value }
    : { valid: false, value: null };
}

function nullableLastRunProjection(value: unknown): {
  valid: boolean;
  value: LastRunProjection | null;
} {
  if (value === null) return { valid: true, value: null };
  return isLastRunProjection(value)
    ? { valid: true, value }
    : { valid: false, value: null };
}

function isActiveRunProjection(value: unknown): value is ActiveRunProjection {
  if (!isRecordWithKeys(value, ["runId"], ["stage", "result"])) return false;
  if (!optionalString(value.runId)) return false;
  if (hasOwn(value, "stage") && !isWorkflowStage(value.stage)) return false;
  return !hasOwn(value, "result") || isRecoveredRunResult(value.result);
}

function isLastRunProjection(value: unknown): value is LastRunProjection {
  if (!isRecordWithKeys(
    value,
    ["runId", "outcome", "stoppedCount", "effects", "finalization"],
    ["cleanupDiagnostic"],
  )) return false;
  return Boolean(optionalString(value.runId))
    && isNonNegativeInteger(value.stoppedCount)
    && isWorkflowOutcome(value.outcome)
    && isEffectCounts(value.effects)
    && isFinalizationAudit(value.finalization)
    && (!hasOwn(value, "cleanupDiagnostic") || isCleanupDiagnostic(value.cleanupDiagnostic));
}

function isWorkflowStage(value: unknown): boolean {
  if (!isRecordWithKeys(value, ["kind", "status"], ["elapsedMs"])) return false;
  const kinds = [
    "contextCapture",
    "recordFinalize",
    "preprocess",
    "transcribe",
    "rewrite",
    "insertPrepare",
    "finalize",
  ];
  return kinds.includes(String(value.kind))
    && ["pending", "started", "completed"].includes(String(value.status))
    && (!hasOwn(value, "elapsedMs") || isNonNegativeNumber(value.elapsedMs));
}

function isWorkflowOutcome(value: unknown): boolean {
  if (!isRecord(value) || Object.keys(value).length !== 1) return false;
  if (hasOwn(value, "completed")) return isCompletedRunResult(value.completed);
  if (hasOwn(value, "empty")) {
    return isRecordWithKeys(value.empty, ["timings"], []) && isRunTimings(value.empty.timings);
  }
  if (hasOwn(value, "failed")) return isFailedRunResult(value.failed);
  if (!hasOwn(value, "cancelled")) return false;
  return isRecordWithKeys(value.cancelled, [], ["recoveredResult"])
    && (!hasOwn(value.cancelled, "recoveredResult")
      || isRecoveredRunResult(value.cancelled.recoveredResult));
}

function isCompletedRunResult(value: unknown): boolean {
  if (!isRecordWithKeys(
    value,
    ["asrText", "finalText", "timings", "insertResult"],
    ["metrics", "warning"],
  )) return false;
  return typeof value.asrText === "string"
    && typeof value.finalText === "string"
    && isRunTimings(value.timings)
    && isInsertResult(value.insertResult)
    && (!hasOwn(value, "metrics") || isTranscriptionMetrics(value.metrics))
    && (!hasOwn(value, "warning") || isWorkflowError(value.warning));
}

function isRecoveredRunResult(value: unknown): boolean {
  if (!isRecordWithKeys(value, ["asrText", "finalText", "timings"], ["metrics"])) return false;
  return typeof value.asrText === "string"
    && typeof value.finalText === "string"
    && isRunTimings(value.timings)
    && (!hasOwn(value, "metrics") || isTranscriptionMetrics(value.metrics));
}

function isFailedRunResult(value: unknown): boolean {
  if (!isRecordWithKeys(
    value,
    ["primaryError", "recoveryErrors", "recordSaved"],
    ["recoveredResult", "protocolContext"],
  )) return false;
  return isWorkflowError(value.primaryError)
    && Array.isArray(value.recoveryErrors)
    && value.recoveryErrors.every(isWorkflowError)
    && typeof value.recordSaved === "boolean"
    && (!hasOwn(value, "recoveredResult") || isRecoveredRunResult(value.recoveredResult))
    && (!hasOwn(value, "protocolContext") || isProtocolContext(value.protocolContext));
}

function isProtocolContext(value: unknown): boolean {
  if (!isRecordWithKeys(value, ["originalVariant"], ["originalError"])) return false;
  return Boolean(optionalString(value.originalVariant))
    && (!hasOwn(value, "originalError") || isWorkflowError(value.originalError));
}

function isWorkflowError(value: unknown): boolean {
  return isRecordWithKeys(value, ["code", "message"], [])
    && Boolean(optionalString(value.code))
    && typeof value.message === "string";
}

function isRunTimings(value: unknown): boolean {
  if (!isRecordWithKeys(
    value,
    ["totalMs"],
    ["recordMs", "preprocessMs", "asrMs", "rewriteMs"],
  )) return false;
  return isNonNegativeNumber(value.totalMs)
    && optionalNonNegativeNumber(value, "recordMs")
    && optionalNonNegativeNumber(value, "preprocessMs")
    && optionalNonNegativeNumber(value, "asrMs")
    && optionalNonNegativeNumber(value, "rewriteMs");
}

function isTranscriptionMetrics(value: unknown): boolean {
  if (!isRecordWithKeys(value, ["rtf", "deviceUsed", "preprocessMs", "asrMs"], [])) return false;
  return isNonNegativeNumber(value.rtf)
    && typeof value.deviceUsed === "string"
    && isNonNegativeNumber(value.preprocessMs)
    && isNonNegativeNumber(value.asrMs);
}

function isInsertResult(value: unknown): boolean {
  if (!isRecordWithKeys(
    value,
    ["copied", "autoPasteAttempted", "autoPasteOk"],
    ["errorCode", "errorMessage"],
  )) return false;
  return typeof value.copied === "boolean"
    && typeof value.autoPasteAttempted === "boolean"
    && typeof value.autoPasteOk === "boolean"
    && optionalNullableString(value, "errorCode")
    && optionalNullableString(value, "errorMessage");
}

function isEffectCounts(value: unknown): boolean {
  if (!isRecordWithKeys(value, ["historyCommitCount", "copyCount", "pasteCount"], [])) return false;
  return isNonNegativeInteger(value.historyCommitCount)
    && isNonNegativeInteger(value.copyCount)
    && isNonNegativeInteger(value.pasteCount);
}

function isFinalizationAudit(value: unknown): boolean {
  return isRecordWithKeys(value, ["commitCount"], []) && isNonNegativeInteger(value.commitCount);
}

function isCleanupDiagnostic(value: unknown): boolean {
  if (!isRecordWithKeys(
    value,
    ["elapsedMs", "gracefulTimedOut", "forceAttempted", "forceSucceeded"],
    ["detail"],
  )) return false;
  return isNonNegativeNumber(value.elapsedMs)
    && typeof value.gracefulTimedOut === "boolean"
    && typeof value.forceAttempted === "boolean"
    && typeof value.forceSucceeded === "boolean"
    && optionalNullableString(value, "detail");
}

function isRecordWithKeys(
  value: unknown,
  required: readonly string[],
  optional: readonly string[],
): value is Record<string, unknown> {
  if (!isRecord(value) || required.some((key) => !hasOwn(value, key))) return false;
  const allowed = new Set([...required, ...optional]);
  return Object.keys(value).every((key) => allowed.has(key));
}

function optionalNonNegativeNumber(value: Record<string, unknown>, key: string): boolean {
  return !hasOwn(value, key) || isNonNegativeNumber(value[key]);
}

function optionalNullableString(value: Record<string, unknown>, key: string): boolean {
  return !hasOwn(value, key) || value[key] === null || typeof value[key] === "string";
}

function isNonNegativeInteger(value: unknown): boolean {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isNonNegativeNumber(value: unknown): boolean {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function statusLabelFromWorkflow(
  view: WorkflowView,
  outcome: "completed" | "empty" | "failed" | "cancelled" | null,
): string {
  if (view.mode === "recording") return "Listening";
  if (view.mode === "processing") return "Creating text";
  if (view.mode === "cancelling") return "Cancelling";
  if (outcome === "completed") return "Text ready";
  if (outcome === "failed") return "Action needed";
  return "Ready";
}

function workflowOutcomeKind(
  view: WorkflowView,
): "completed" | "empty" | "failed" | "cancelled" | null {
  const outcome = valueAt(view.lastRun, "outcome");
  if (!isRecord(outcome)) return null;
  if (hasOwn(outcome, "completed")) return "completed";
  if (hasOwn(outcome, "empty")) return "empty";
  if (hasOwn(outcome, "failed")) return "failed";
  if (hasOwn(outcome, "cancelled")) return "cancelled";
  return null;
}

function valueAt(root: unknown, ...path: string[]): unknown {
  let value = root;
  for (const key of path) {
    if (!isRecord(value)) return undefined;
    value = value[key];
  }
  return value;
}

function isRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasOwn(value: object, key: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}
