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

export const defaultTauriGateway: TauriGateway = {
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
