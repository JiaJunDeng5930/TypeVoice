import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { compactDetail, extractErrorCode } from "../domain/diagnostic";

export type Unlisten = () => void;

export interface TauriGateway {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(eventName: string, handler: (payload: T) => void | Promise<void>): Promise<Unlisten>;
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

export const defaultTauriGateway: TauriGateway = tauriGateway;
