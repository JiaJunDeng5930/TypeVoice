import type { UiEvent } from "../types";

export function appendTranscript(base: string, next: string): string {
  const cleanNext = next.trim();
  if (!cleanNext) return base;
  const cleanBase = base.trimEnd();
  if (!cleanBase) return cleanNext;
  return `${cleanBase}\n${cleanNext}`;
}

export function isTranscriptionPartialForRun(
  event: UiEvent,
  activeRunId: string | null,
): boolean {
  return Boolean(activeRunId && event.taskId === activeRunId);
}

export function textFromTranscriptionPartial(
  event: UiEvent,
  activeRunId: string | null,
): string {
  if (!isTranscriptionPartialForRun(event, activeRunId)) return "";
  if (!event.payload || typeof event.payload !== "object") return "";
  const payload = event.payload as Record<string, unknown>;
  return typeof payload.text === "string" ? payload.text : "";
}
