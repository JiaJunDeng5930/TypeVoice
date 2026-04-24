import { defaultTauriGateway, type TauriGateway } from "./runtimePorts";
import type {
  HistoryItem,
  InsertResult,
  RecordTranscribeStartResult,
  RewriteResult,
  RuntimePythonStatus,
  RuntimeToolchainStatus,
  TranscriptionResult,
  UiEvent,
} from "../types";

export type BackendClient = {
  recordTranscribeStart(req: { taskId?: string | null }): Promise<RecordTranscribeStartResult>;
  recordTranscribeStop(req: { sessionId: string }): Promise<TranscriptionResult>;
  recordTranscribeCancel(req: {
    sessionId?: string | null;
    transcriptId?: string | null;
  }): Promise<void>;
  rewriteText(req: {
    transcriptId: string;
    text: string;
    templateId?: string | null;
  }): Promise<RewriteResult>;
  insertText(req: { transcriptId?: string | null; text: string }): Promise<InsertResult>;
  abortPendingTask(taskId: string): Promise<void>;
  overlaySetState(state: {
    visible: boolean;
    status: string;
    detail?: string | null;
    ts_ms: number;
  }): Promise<void>;
  logUiEvent(req: Record<string, unknown>): Promise<void>;
  runtimeToolchainStatus(): Promise<RuntimeToolchainStatus>;
  runtimePythonStatus(): Promise<RuntimePythonStatus>;
  historyList(req: { limit: number; beforeMs?: number | null }): Promise<HistoryItem[]>;
  listenUiEvent(handler: (event: UiEvent) => void | Promise<void>): Promise<() => void>;
  listenHotkeyRecord<T>(handler: (event: T) => void | Promise<void>): Promise<() => void>;
};

export function createBackendClient(gateway: TauriGateway = defaultTauriGateway): BackendClient {
  return {
    recordTranscribeStart(req) {
      return gateway.invoke<RecordTranscribeStartResult>("record_transcribe_start", { req });
    },
    recordTranscribeStop(req) {
      return gateway.invoke<TranscriptionResult>("record_transcribe_stop", { req });
    },
    recordTranscribeCancel(req) {
      return gateway.invoke<void>("record_transcribe_cancel", { req });
    },
    rewriteText(req) {
      return gateway.invoke<RewriteResult>("rewrite_text", { req });
    },
    insertText(req) {
      return gateway.invoke<InsertResult>("insert_text", { req });
    },
    abortPendingTask(taskId) {
      return gateway.invoke<void>("abort_pending_task", { taskId });
    },
    overlaySetState(state) {
      return gateway.invoke<void>("overlay_set_state", { state });
    },
    logUiEvent(req) {
      return gateway.invoke<void>("ui_log_event", { req });
    },
    runtimeToolchainStatus() {
      return gateway.invoke<RuntimeToolchainStatus>("runtime_toolchain_status");
    },
    runtimePythonStatus() {
      return gateway.invoke<RuntimePythonStatus>("runtime_python_status");
    },
    historyList(req) {
      return gateway.invoke<HistoryItem[]>("history_list", req);
    },
    listenUiEvent(handler) {
      return gateway.listen<UiEvent>("ui_event", handler);
    },
    listenHotkeyRecord<T>(handler: (event: T) => void | Promise<void>) {
      return gateway.listen<T>("tv_hotkey_record", handler);
    },
  };
}
