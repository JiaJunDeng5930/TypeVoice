import { useEffect, useMemo, useState } from "react";
import {
  defaultTauriGateway,
  type TauriGateway,
} from "../infra/runtimePorts";
import { createBackendClient, type BackendClient } from "../infra/backendClient";
import { buildDiagnostic, buildUiEventDiagnostic, userMessageFromDiagnostic } from "../domain/diagnostic";
import {
  EMPTY_WORKFLOW_VIEW,
  primaryActionLabel,
  workflowPhaseName,
  workflowViewFromPayload,
} from "../domain/workflowView";
import type {
  RuntimeToolchainStatus,
  Settings,
  TranscriptionMetrics,
  WorkflowCommand,
  WorkflowView,
} from "../types";
import { IconStart, IconStop, IconTranscribing } from "../ui/icons";

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
  gateway?: TauriGateway;
  backend?: BackendClient;
};

export function MainScreen({
  settings: _settings,
  pushToast,
  onHistoryChanged,
  gateway = defaultTauriGateway,
  backend,
}: Props) {
  const client = useMemo(() => backend || createBackendClient(gateway), [backend, gateway]);
  const [workflow, setWorkflow] = useState<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const [hover, setHover] = useState(false);
  const [liveTranscript, setLiveTranscript] = useState("");

  useEffect(() => {
    (async () => {
      const view = await client.workflowSnapshot();
      setWorkflow(view);
    })().catch((err) => {
      const diag = buildDiagnostic(err, "WORKFLOW STATE FAILED");
      pushToast(diag.title, "danger");
    });
  }, [client, pushToast]);

  useEffect(() => {
    (async () => {
      try {
        const runtime = await client.runtimeToolchainStatus() as RuntimeToolchainStatus;
        if (!runtime.ready) pushToast("Local audio tools need repair", "danger");
      } catch {
      }
    })();
  }, [client, pushToast]);

  useEffect(() => {
    let cancelled = false;
    const unlistenFns: Array<() => void> = [];
    const trackUnlisten = (fn: () => void) => {
      if (cancelled) {
        try {
          fn();
        } catch {
        }
        return;
      }
      unlistenFns.push(fn);
    };

    (async () => {
      const unlistenUiEvent = await client.listenUiEvent(async (ev) => {
        if (!ev || ev.kind === "audio.level") return;
        if (ev.kind === "transcription.partial") {
          const partial = transcriptionPartialPayload(ev.payload);
          if (partial?.text) setLiveTranscript(partial.text);
          return;
        }
        if (ev.kind === "workflow.state") {
          const next = workflowViewFromPayload(ev.payload);
          if (next) {
            setWorkflow(next);
            const phase = String(next.phase || "").toLowerCase();
            if (phase === "recording") setLiveTranscript("");
          }
          return;
        }
        if (ev.status === "failed") {
          const diag = buildUiEventDiagnostic(ev, "Speech recognition failed");
          if (ev.stage === "Transcribe") {
            const transcriptId = optionalString(ev.taskId);
            if (transcriptId) {
              try {
                const next = await client.workflowReportAsrFailed({
                  transcriptId,
                  code: optionalString(ev.errorCode) || "E_TRANSCRIBE_FAILED",
                  message: ev.message,
                });
                setWorkflow(next);
              } catch (err) {
                const diag = buildDiagnostic(err, "WORKFLOW EVENT FAILED");
                pushToast(diag.title, "danger");
              }
            }
          }
          pushToast(diag.title, "danger");
          return;
        }
        if (ev.status === "cancelled") {
          pushToast("CANCELLED", "default");
          return;
        }
        if (ev.kind === "transcription.empty") {
          const transcriptId = optionalString(ev.taskId);
          if (transcriptId) {
            try {
              const next = await client.workflowReportAsrEmpty({ transcriptId });
              setWorkflow(next);
            } catch (err) {
              const diag = buildDiagnostic(err, "WORKFLOW EVENT FAILED");
              pushToast(diag.title, "danger");
              return;
            }
          }
          setLiveTranscript("");
          pushToast("未检测到语音", "default");
          return;
        }
        if (ev.kind === "transcription.completed") {
          const completed = transcriptionCompletedPayload(ev.payload);
          if (!completed) {
            pushToast("Speech recognition failed", "danger");
            return;
          }
          try {
            const next = await client.workflowReportAsrCompleted({
              transcriptId: completed.transcriptId,
              text: completed.asrText,
              metrics: completed.metrics,
            });
            setWorkflow(next);
            setLiveTranscript("");
            pushToast("Text ready", "ok");
            onHistoryChanged();
          } catch (err) {
            const diag = buildDiagnostic(err, "WORKFLOW EVENT FAILED");
            pushToast(diag.title, "danger");
          }
          return;
        }
        if (ev.kind === "rewrite.completed") {
          pushToast("Text improved", "ok");
          onHistoryChanged();
          return;
        }
        if (ev.kind === "insertion.completed") {
          const inserted = insertionPayload(ev.payload);
          if (inserted?.autoPasteAttempted && !inserted.autoPasteOk) {
            pushToast("Text could not be pasted", "danger");
          } else {
            pushToast(inserted?.autoPasteAttempted ? "Text pasted" : "Text copied", "ok");
          }
        }
      });
      trackUnlisten(unlistenUiEvent);
    })().catch((err) => {
      const diag = buildDiagnostic(err, "UI EVENT LISTEN FAILED");
      pushToast(diag.title, "danger");
    });

    return () => {
      cancelled = true;
      for (const fn of unlistenFns) {
        try {
          fn();
        } catch {
        }
      }
    };
  }, [client, onHistoryChanged, pushToast]);

  async function sendWorkflowCommand(command: WorkflowCommand) {
    try {
      const next = await client.workflowCommand({ command });
      setWorkflow(next);
      if (command === "copyLast") {
        pushToast("Text copied", "ok");
      }
    } catch (err) {
      const diag = buildDiagnostic(err, commandErrorTitle(command));
      pushToast(diag.title, "danger");
      try {
        const refreshed = await client.workflowSnapshot();
        setWorkflow(refreshed);
      } catch (refreshErr) {
        const refreshDiag = buildDiagnostic(refreshErr, "WORKFLOW STATE FAILED");
        pushToast(refreshDiag.title, "danger");
      }
    }
  }

  const phase = workflowPhaseName(workflow.phase);
  const hint = primaryActionLabel(workflow.primaryLabel || "START");
  const streamText = phase === "recording" || phase === "transcribing" ? liveTranscript : "";
  const statusLabel = phase === "idle" ? "Ready" : hint;
  const diagnosticMessage = userMessageFromDiagnostic(workflow.diagnosticCode, workflow.diagnosticLine);

  return (
    <div className="pageSurface mainSurface">
      <div className="voiceDock">
        <button
          type="button"
          className={`mainButton ${phase === "transcribing" ? "isBusy" : ""}`}
          onClick={() => void sendWorkflowCommand("primary")}
          disabled={workflow.primaryDisabled}
          onMouseEnter={() => setHover(true)}
          onMouseLeave={() => setHover(false)}
          onFocus={() => setHover(true)}
          onBlur={() => setHover(false)}
          aria-label={hint}
          title={hint}
        >
          {phase === "idle" || phase === "transcribed" || phase === "rewritten" || phase === "cancelled" || phase === "failed" ? (
            <IconStart size={42} tone="accent" />
          ) : phase === "recording" ? (
            <IconStop size={42} tone="accent" />
          ) : (
            <IconTranscribing size={42} tone="accent" />
          )}
          <span className="mainButtonText">{hint}</span>
        </button>

        <div className="mainHint">{hover ? hint : statusLabel}</div>
      </div>

      <div className="resultSheet">
        <div className="resultHeader">
          <div className="sectionTitle">CURRENT TRANSCRIPT</div>
          <span className="resultStatus">Ready</span>
        </div>

        <div className="streamCanvas" aria-live="polite">
          <div className={`streamText ${streamText.trim() ? "" : "isEmpty"}`}>
            {streamText.trim() || "Start recording to see live transcription here."}
          </div>
        </div>

        <div
          className={`mainDiag ${workflow.diagnosticLine ? "isVisible" : ""}`}
          aria-hidden={!workflow.diagnosticLine}
        >
          {diagnosticMessage || "Ready"}
        </div>

        <div className="mainActions">
          <button
            type="button"
            className="mainActionButton"
            onClick={() => void sendWorkflowCommand("rewriteLast")}
            disabled={true}
          >
            Improve
          </button>
          <button
            type="button"
            className="mainActionButton"
            onClick={() => void sendWorkflowCommand("insertLast")}
            disabled={true}
          >
            Paste
          </button>
        </div>
      </div>
    </div>
  );
}

function insertionPayload(payload: unknown): {
  autoPasteAttempted: boolean;
  autoPasteOk: boolean;
  errorCode?: string | null;
} | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  return {
    autoPasteAttempted: raw.autoPasteAttempted === true,
    autoPasteOk: raw.autoPasteOk === true,
    errorCode: optionalString(raw.errorCode),
  };
}

function transcriptionPartialPayload(payload: unknown): { text: string } | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  return { text: String(raw.text || "") };
}

function transcriptionCompletedPayload(payload: unknown): {
  transcriptId: string;
  asrText: string;
  metrics: TranscriptionMetrics;
} | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  const transcriptId = optionalString(raw.transcriptId);
  const asrText = String(raw.asrText || "");
  const metrics = metricsPayload(raw.metrics);
  if (!transcriptId || !asrText.trim() || !metrics) return null;
  return { transcriptId, asrText, metrics };
}

function metricsPayload(payload: unknown): TranscriptionMetrics | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  return {
    rtf: optionalNumber(raw.rtf) || 0,
    deviceUsed: String(raw.deviceUsed || ""),
    preprocessMs: optionalNumber(raw.preprocessMs) || 0,
    asrMs: optionalNumber(raw.asrMs) || 0,
  };
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function optionalNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function commandErrorTitle(command: WorkflowCommand): string {
  if (command === "rewriteLast") return "Text improvement failed";
  if (command === "insertLast") return "Text could not be pasted";
  if (command === "copyLast") return "Copy failed";
  if (command === "cancel") return "Cancel failed";
  return "Recording action failed";
}
