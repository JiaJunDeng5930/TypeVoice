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

export const defaultTauriGateway: TauriGateway = {
  async invoke<T>(command: string, args?: Record<string, unknown>) {
    return tauriInvoke<T>(command, args);
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
