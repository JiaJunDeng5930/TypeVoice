import { userMessageFromDiagnosticLine } from "./diagnostic";
import type { WorkflowView } from "../types";

export type WorkflowPhaseName =
  | "idle"
  | "recording"
  | "transcribing"
  | "transcribed"
  | "rewriting"
  | "rewritten"
  | "inserting"
  | "cancelled"
  | "failed";

export type OverlayTone = "default" | "ok" | "danger";

export type OverlayViewState = {
  visible: boolean;
  status: string;
  detail: string | null;
  tone: OverlayTone;
};

export const EMPTY_WORKFLOW_VIEW: WorkflowView = {
  phase: "idle",
  taskId: null,
  recordingSessionId: null,
  lastTranscriptId: null,
  lastAsrText: "",
  lastText: "",
  lastCreatedAtMs: null,
  diagnosticCode: null,
  diagnosticLine: "",
  primaryLabel: "START",
  primaryDisabled: false,
  canRewrite: false,
  canInsert: false,
  canCopy: false,
};

export function workflowPhaseName(raw: string | null | undefined): WorkflowPhaseName {
  const value = String(raw || "idle").trim().toLowerCase();
  if (
    value === "recording"
    || value === "transcribing"
    || value === "transcribed"
    || value === "rewriting"
    || value === "rewritten"
    || value === "inserting"
    || value === "cancelled"
    || value === "failed"
  ) {
    return value;
  }
  return "idle";
}

export function workflowViewFromPayload(payload: unknown): WorkflowView | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  return {
    phase: workflowPhaseName(String(raw.phase || "idle")),
    taskId: optionalString(raw.taskId),
    recordingSessionId: optionalString(raw.recordingSessionId),
    lastTranscriptId: optionalString(raw.lastTranscriptId),
    lastAsrText: String(raw.lastAsrText || ""),
    lastText: String(raw.lastText || ""),
    lastCreatedAtMs: optionalNumber(raw.lastCreatedAtMs),
    diagnosticCode: optionalString(raw.diagnosticCode),
    diagnosticLine: String(raw.diagnosticLine || ""),
    primaryLabel: String(raw.primaryLabel || "START"),
    primaryDisabled: raw.primaryDisabled === true,
    canRewrite: raw.canRewrite === true,
    canInsert: raw.canInsert === true,
    canCopy: raw.canCopy === true,
  };
}

export function overlayViewFromWorkflow(view: WorkflowView): OverlayViewState {
  const phase = workflowPhaseName(view.phase);
  const status = statusLabelFromPhase(phase);
  const visible = isOverlayVisiblePhase(phase);
  const detail = view.diagnosticLine
    ? userMessageFromDiagnosticLine(view.diagnosticLine)
    : null;
  return {
    visible,
    status,
    detail,
    tone: toneFromOverlayStatus(status),
  };
}

export function statusLabelFromPhase(phase: string): string {
  const name = workflowPhaseName(phase);
  if (name === "recording") return "Listening";
  if (name === "transcribing") return "Creating text";
  if (name === "rewriting") return "Improving text";
  if (name === "rewritten") return "Text improved";
  if (name === "inserting") return "Pasting text";
  if (name === "transcribed") return "Text ready";
  if (name === "failed") return "Action needed";
  return "Ready";
}

export function toneFromOverlayStatus(status: string): OverlayTone {
  if (status === "Action needed") return "danger";
  if (status === "Text improved" || status === "Text ready") return "ok";
  return "default";
}

export function isActiveWorkflowPhase(phase: string): boolean {
  const name = workflowPhaseName(phase);
  return name === "recording"
    || name === "transcribing"
    || name === "rewriting"
    || name === "inserting";
}

export function canTogglePrimaryFromOverlay(phase: string): boolean {
  const name = workflowPhaseName(phase);
  return name === "idle"
    || name === "recording"
    || name === "cancelled"
    || name === "failed";
}

export function primaryActionLabel(raw: string): string {
  const label = raw.trim().toUpperCase();
  if (label === "START") return "Start";
  if (label === "STOP") return "Stop";
  if (label === "TRANSCRIBING") return "Creating text";
  if (label === "TRANSCRIBED") return "Text ready";
  if (label === "REWRITING") return "Improving text";
  if (label === "REWRITTEN") return "Text improved";
  if (label === "INSERTING") return "Pasting text";
  return raw.trim() || "Start";
}

function isOverlayVisiblePhase(phase: WorkflowPhaseName): boolean {
  if (phase === "recording") return true;
  if (phase === "transcribing") return true;
  if (phase === "rewriting") return true;
  if (phase === "inserting") return true;
  if (phase === "transcribed" || phase === "rewritten") return true;
  return false;
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function optionalNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
