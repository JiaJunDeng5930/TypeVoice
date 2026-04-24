import { defaultTauriGateway, type TauriGateway } from "./runtimePorts";
import type {
  HistoryItem,
  RuntimePythonStatus,
  RuntimeToolchainStatus,
  UiEvent,
  WorkflowCommand,
  WorkflowView,
} from "../types";

export type BackendClient = {
  workflowSnapshot(): Promise<WorkflowView>;
  workflowCommand(req: {
    command: WorkflowCommand;
    taskId?: string | null;
  }): Promise<WorkflowView>;
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
};

export function createBackendClient(gateway: TauriGateway = defaultTauriGateway): BackendClient {
  return {
    workflowSnapshot() {
      return gateway.invoke<WorkflowView>("workflow_snapshot");
    },
    workflowCommand(req) {
      return gateway.invoke<WorkflowView>("workflow_command", { req });
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
  };
}
