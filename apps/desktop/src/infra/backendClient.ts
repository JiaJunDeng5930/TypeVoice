import { defaultTauriGateway, type TauriGateway } from "./runtimePorts";
import type {
  HistoryItem,
  RuntimeToolchainStatus,
  UiEvent,
  WorkflowApplyEventRequest,
  WorkflowCommand,
  WorkflowView,
  RewriteResult,
  InsertResult,
} from "../types";

export type BackendClient = {
  workflowSnapshot(): Promise<WorkflowView>;
  workflowCommand(req: {
    command: WorkflowCommand;
    taskId?: string | null;
  }): Promise<WorkflowView>;
  workflowApplyEvent(req: WorkflowApplyEventRequest): Promise<WorkflowView>;
  rewriteText(req: { transcriptId: string; text: string; templateId?: string | null }): Promise<RewriteResult>;
  insertOverlayText(req: { transcriptId?: string | null; text: string }): Promise<InsertResult>;
  overlaySetState(state: {
    visible: boolean;
    status: string;
    detail?: string | null;
    ts_ms: number;
  }): Promise<void>;
  overlayResize(req: { width: number; height: number }): Promise<void>;
  logUiEvent(req: Record<string, unknown>): Promise<void>;
  runtimeToolchainStatus(): Promise<RuntimeToolchainStatus>;
  historyList(req: { limit: number; beforeMs?: number | null }): Promise<HistoryItem[]>;
  listenUiEvent(handler: (event: UiEvent) => void | Promise<void>): Promise<() => void>;
};

export function createBackendClient(gateway: TauriGateway = defaultTauriGateway): BackendClient {
  return {
    workflowSnapshot() {
      return gateway.invoke<WorkflowView>("workflow_snapshot");
    },
    workflowCommand(req) {
      return gateway.invoke<WorkflowView>("workflow_command", { req });
    },
    workflowApplyEvent(req) {
      return gateway.invoke<WorkflowView>("workflow_apply_event", { req });
    },
    rewriteText(req) {
      return gateway.invoke<RewriteResult>("rewrite_text", { req });
    },
    insertOverlayText(req) {
      return gateway.invoke<InsertResult>("overlay_insert_text", { req });
    },
    overlaySetState(state) {
      return gateway.invoke<void>("overlay_set_state", { state });
    },
    overlayResize(req) {
      return gateway.invoke<void>("overlay_resize", { req });
    },
    logUiEvent(req) {
      return gateway.invoke<void>("ui_log_event", { req });
    },
    runtimeToolchainStatus() {
      return gateway.invoke<RuntimeToolchainStatus>("runtime_toolchain_status");
    },
    historyList(req) {
      return gateway.invoke<HistoryItem[]>("history_list", req);
    },
    listenUiEvent(handler) {
      return gateway.listen<UiEvent>("ui_event", handler);
    },
  };
}
