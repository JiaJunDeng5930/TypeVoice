import type { UiEvent } from "../types";

export function appendTranscript(base: string, next: string): string {
  const cleanNext = next.trim();
  if (!cleanNext) return base;
  const cleanBase = base.trimEnd();
  if (!cleanBase) return cleanNext;
  return `${cleanBase}\n${cleanNext}`;
}

export function textFromTranscriptionPartial(
  event: UiEvent,
  activeRunId: string | null,
): string {
  if (!activeRunId || event.taskId !== activeRunId) return "";
  if (!event.payload || typeof event.payload !== "object") return "";
  const payload = event.payload as Record<string, unknown>;
  return String(payload.text || "");
}
