import { useCallback, useEffect, useRef, useState } from "react";
import { buildDiagnostic, buildUiEventDiagnostic, userMessageFromDiagnostic } from "../domain/diagnostic";
import {
  EMPTY_WORKFLOW_VIEW,
  isWorkflowCommandReply,
  primaryActionLabel,
  shouldAcceptWorkflowProjection,
  workflowDiagnostic,
  workflowPresentationPhase,
  workflowProjectionFromPayload,
  workflowProjectionRevision,
  workflowViewFromPayload,
} from "../domain/workflowView";
import { defaultTauriGateway } from "../infra/runtimePorts";
import type {
  RuntimeToolchainStatus,
  Settings,
  UiEvent,
  WorkflowCommandReply,
  WorkflowCommandRequest,
  WorkflowView,
} from "../types";
import { IconStart, IconStop, IconTranscribing } from "../ui/icons";
import { PixelButton } from "../ui/PixelButton";

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
};

export function MainScreen({
  pushToast,
  onHistoryChanged,
}: Props) {
  const [workflow, setWorkflow] = useState<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const workflowRef = useRef<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const latestWorkflowRevisionRef = useRef<number | null>(null);
  const observedLastRunIdRef = useRef<string | null>(null);

  const acceptWorkflowView = useCallback((next: WorkflowView): boolean => {
    if (!shouldAcceptWorkflowProjection(latestWorkflowRevisionRef.current, next)) return false;
    latestWorkflowRevisionRef.current = workflowProjectionRevision(next);
    workflowRef.current = next;
    setWorkflow(next);

    const lastRunId = next.lastRun?.runId ?? null;
    if (lastRunId && lastRunId !== observedLastRunIdRef.current) {
      observedLastRunIdRef.current = lastRunId;
      onHistoryChanged();
    }
    return true;
  }, [onHistoryChanged]);

  const refreshWorkflowSnapshot = useCallback(async () => {
    const payload = await defaultTauriGateway.invoke<unknown>("workflow_snapshot");
    const next = workflowProjectionFromPayload(payload);
    if (!next) throw new Error("workflow_snapshot returned a malformed projection");
    acceptWorkflowView(next);
  }, [acceptWorkflowView]);

  const processCommandReply = useCallback((reply: WorkflowCommandReply) => {
    switch (reply.disposition) {
      case "applied":
      case "noOp":
        break;
      case "cancelTooLate":
        pushToast("That action is already finishing", "default");
        break;
    }
    acceptWorkflowView(reply.view);
  }, [acceptWorkflowView, pushToast]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    void (async () => {
      const stop = await defaultTauriGateway.listen<UiEvent>("ui_event", (event) => {
        if (cancelled || !event || event.kind === "audio.level") return;
        if (event.kind === "workflow.state") {
          const next = workflowProjectionFromPayload(event.payload);
          if (next) acceptWorkflowView(next);
          return;
        }
        handleDisplayEvent(event, pushToast);
      });

      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      await refreshWorkflowSnapshot();
    })().catch((error) => {
      if (cancelled) return;
      const diagnostic = buildDiagnostic(error, "Live recording updates unavailable");
      pushToast(diagnostic.title, "danger");
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [acceptWorkflowView, pushToast, refreshWorkflowSnapshot]);

  useEffect(() => {
    void (async () => {
      try {
        const runtime = await defaultTauriGateway.invoke<RuntimeToolchainStatus>("runtime_toolchain_status");
        if (!runtime.ready) pushToast("Local audio tools need repair", "danger");
      } catch {
      }
    })();
  }, [pushToast]);

  async function sendWorkflowCommand(request: WorkflowCommandRequest) {
    try {
      const payload = await defaultTauriGateway.invoke<unknown>("workflow_command", { req: request });
      const parsed = workflowViewFromPayload(payload);
      if (!parsed || !isWorkflowCommandReply(parsed)) {
        throw new Error("workflow_command returned a malformed reply");
      }
      processCommandReply(parsed);
    } catch (error) {
      const diagnostic = buildDiagnostic(error, commandErrorTitle(request.command));
      pushToast(diagnostic.title, "danger");
      try {
        await refreshWorkflowSnapshot();
      } catch (refreshError) {
        const refreshDiagnostic = buildDiagnostic(refreshError, "Recording status unavailable");
        pushToast(refreshDiagnostic.title, "danger");
      }
    }
  }

  const presentationPhase = workflowPresentationPhase(workflow);
  const buttonLabel = primaryActionLabel(workflow.primaryLabel);
  const cancelTargetRunId = workflow.cancelEnabled ? workflow.activeRun?.runId ?? null : null;
  const diagnostic = workflowDiagnostic(workflow);
  const diagnosticMessage = userMessageFromDiagnostic(diagnostic.code, diagnostic.message);

  return (
    <div className="pageSurface mainSurface" aria-live="polite">
      <button
        type="button"
        className={`mainButton status-${presentationPhase}`}
        onClick={() => void sendWorkflowCommand({
          command: "primary",
          actionKey: workflowRef.current.actionKey,
        })}
        disabled={workflow.primaryDisabled}
        aria-label={buttonLabel}
        aria-busy={workflow.mode === "processing" || workflow.mode === "cancelling"}
        title={buttonLabel}
      >
        <span className="mainButtonIcon" aria-hidden="true">
          {workflow.mode === "ready" ? (
            <IconStart size={28} tone="accent" />
          ) : workflow.mode === "recording" ? (
            <IconStop size={28} tone="accent" />
          ) : (
            <IconTranscribing size={28} tone="accent" />
          )}
        </span>
        <strong>{buttonLabel}</strong>
      </button>

      {cancelTargetRunId ? (
        <PixelButton
          tone="danger"
          onClick={() => void sendWorkflowCommand({
            command: "cancel",
            targetRunId: cancelTargetRunId,
          })}
          title="Cancel the current recording"
        >
          Cancel
        </PixelButton>
      ) : null}

      {diagnostic.code || diagnostic.message ? (
        <div className="mainDiag isVisible" role="alert">
          {diagnostic.code ? <span>{diagnostic.code}</span> : null}
          {diagnosticMessage}
        </div>
      ) : null}
    </div>
  );
}

function handleDisplayEvent(
  event: UiEvent,
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void,
) {
  if (event.kind === "transcription.partial") return;
  if (event.kind === "workflow.task.failed" || event.status === "failed") {
    const diagnostic = buildUiEventDiagnostic(event, failureTitleFromStage(event.stage));
    pushToast(diagnostic.title, "danger");
    return;
  }
  if (event.status === "cancelled") {
    pushToast("Cancelled", "default");
    return;
  }
  if (event.kind === "transcription.empty") {
    pushToast("No speech detected", "default");
    return;
  }
  if (event.kind === "transcription.completed") {
    pushToast("Text ready", "ok");
    return;
  }
  if (event.kind === "rewrite.completed") {
    pushToast("Text improved", "ok");
    return;
  }
  if (event.kind === "insertion.completed") {
    const inserted = insertionPayload(event.payload);
    if (inserted?.autoPasteAttempted && !inserted.autoPasteOk) {
      pushToast("Text could not be pasted", "danger");
    } else {
      pushToast(inserted?.autoPasteAttempted ? "Text pasted" : "Text copied", "ok");
    }
  }
}

function insertionPayload(payload: unknown): {
  autoPasteAttempted: boolean;
  autoPasteOk: boolean;
} | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  return {
    autoPasteAttempted: raw.autoPasteAttempted === true,
    autoPasteOk: raw.autoPasteOk === true,
  };
}

function failureTitleFromStage(stage: string | null | undefined): string {
  if (stage === "Rewrite") return "Text improvement failed";
  if (stage === "Insert") return "Text could not be pasted";
  return "Speech recognition failed";
}

function commandErrorTitle(command: WorkflowCommandRequest["command"]): string {
  return command === "cancel" ? "Cancel failed" : "Recording action failed";
}
