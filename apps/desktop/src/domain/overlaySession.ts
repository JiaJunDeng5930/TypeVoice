import type { UiEvent } from "../types";

export type OverlayKeyAction = "rewrite" | "insert" | "newline" | "none";

export function appendTranscript(base: string, next: string): string {
  const cleanNext = next.trim();
  if (!cleanNext) return base;
  const cleanBase = base.trimEnd();
  if (!cleanBase) return cleanNext;
  return `${cleanBase}\n${cleanNext}`;
}

export function textFromTranscriptionPartial(event: UiEvent): string {
  if (!event.payload || typeof event.payload !== "object") return "";
  const payload = event.payload as Record<string, unknown>;
  return String(payload.text || "");
}

export function textFromTranscriptionCompleted(event: UiEvent): {
  transcriptId: string | null;
  asrText: string;
} {
  if (!event.payload || typeof event.payload !== "object") {
    return { transcriptId: null, asrText: "" };
  }
  const payload = event.payload as Record<string, unknown>;
  return {
    transcriptId: optionalString(payload.transcriptId),
    asrText: String(payload.asrText || ""),
  };
}

export function textFromRewriteCompleted(event: UiEvent): {
  transcriptId: string | null;
  finalText: string;
} {
  if (!event.payload || typeof event.payload !== "object") {
    return { transcriptId: null, finalText: "" };
  }
  const payload = event.payload as Record<string, unknown>;
  return {
    transcriptId: optionalString(payload.transcriptId),
    finalText: String(payload.finalText || ""),
  };
}

export function overlayKeyAction(event: Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "metaKey">): OverlayKeyAction {
  if (event.key !== "Enter") return "none";
  if (event.shiftKey) return "newline";
  if (event.ctrlKey || event.metaKey) return "insert";
  return "rewrite";
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}
