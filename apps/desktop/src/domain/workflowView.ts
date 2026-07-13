import { userMessageFromDiagnosticLine } from "./diagnostic.ts";
import type {
  WorkflowCommandDisposition,
  WorkflowCommandReply,
  WorkflowMode,
  WorkflowRunProjection,
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
  const revision = candidate.revision;
  return isRevision(revision) ? revision : null;
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
  const mode = workflowMode(raw.mode);
  if (!mode || !isRevision(raw.revision)) return null;
  if (typeof raw.actionKey !== "string" || !raw.actionKey.trim()) return null;
  if (typeof raw.primaryLabel !== "string" || !raw.primaryLabel.trim()) return null;
  if (typeof raw.primaryDisabled !== "boolean" || typeof raw.cancelEnabled !== "boolean") {
    return null;
  }
  if (!hasOwn(raw, "activeRun") || !hasOwn(raw, "lastRun")) return null;

  const activeRun = nullableRunProjection(raw.activeRun);
  const lastRun = nullableRunProjection(raw.lastRun);
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

function nullableRunProjection(value: unknown): {
  valid: boolean;
  value: WorkflowRunProjection | null;
} {
  if (value === null) return { valid: true, value: null };
  if (!isRecord(value)) return { valid: false, value: null };
  const runId = optionalString(value.runId);
  if (!runId) return { valid: false, value: null };
  return { valid: true, value: { ...value, runId } };
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
