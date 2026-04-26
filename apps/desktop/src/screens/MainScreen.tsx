import { useEffect, useMemo, useState } from "react";
import {
  defaultTauriGateway,
  type TauriGateway,
} from "../infra/runtimePorts";
import { createBackendClient, type BackendClient } from "../infra/backendClient";
import { buildDiagnostic } from "../domain/diagnostic";
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

const EMPTY_WORKFLOW_VIEW: WorkflowView = {
  phase: "idle",
  taskId: null,
  recordingSessionId: null,
  lastTranscriptId: null,
  lastAsrText: "",
  lastText: "",
  lastCreatedAtMs: null,
  diagnosticCode: null,
  diagnosticLine: "",
  primaryLabel: "START",
  primaryDisabled: false,
  canRewrite: false,
  canInsert: false,
  canCopy: false,
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
        if (!runtime.ready) pushToast("TOOLCHAIN NOT READY", "danger");
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
          const title = ev.stage === "Rewrite"
            ? "REWRITE FAILED"
            : ev.stage === "Insert"
              ? "INSERT FAILED"
              : "TRANSCRIBE FAILED";
          pushToast(title, "danger");
          return;
        }
        if (ev.status === "cancelled") {
          pushToast("CANCELLED", "default");
          return;
        }
        if (ev.kind === "transcription.completed") {
          const completed = transcriptionCompletedPayload(ev.payload);
          if (!completed) {
            pushToast("TRANSCRIBE FAILED", "danger");
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
            pushToast("TRANSCRIBED", "ok");
            onHistoryChanged();
          } catch (err) {
            const diag = buildDiagnostic(err, "WORKFLOW EVENT FAILED");
            pushToast(diag.title, "danger");
          }
          return;
        }
        if (ev.kind === "rewrite.completed") {
          pushToast("REWRITTEN", "ok");
          onHistoryChanged();
          return;
        }
        if (ev.kind === "insertion.completed") {
          const inserted = insertionPayload(ev.payload);
          if (inserted?.autoPasteAttempted && !inserted.autoPasteOk) {
            pushToast(`AUTO PASTE FAILED: ${inserted.errorCode || "E_EXPORT_PASTE_FAILED"}`, "danger");
          } else {
            pushToast(inserted?.autoPasteAttempted ? "COPIED + PASTED" : "COPIED", "ok");
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
        pushToast("COPIED", "ok");
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

  const phase = String(workflow.phase || "idle").toLowerCase();
  const hint = workflow.primaryLabel || "START";
  const streamText = phase === "recording" || phase === "transcribing" ? liveTranscript : "";
  const statusLabel = phase === "idle" ? "READY" : hint;

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
          <span className="resultStatus">READY</span>
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
          {workflow.diagnosticLine || "NO ERRORS"}
        </div>

        <div className="mainActions">
          <button
            type="button"
            className="mainActionButton"
            onClick={() => void sendWorkflowCommand("rewriteLast")}
            disabled={true}
          >
            REWRITE
          </button>
          <button
            type="button"
            className="mainActionButton"
            onClick={() => void sendWorkflowCommand("insertLast")}
            disabled={true}
          >
            INSERT
          </button>
        </div>
      </div>
    </div>
  );
}

function workflowViewFromPayload(payload: unknown): WorkflowView | null {
  if (!payload || typeof payload !== "object") return null;
  const raw = payload as Record<string, unknown>;
  return {
    phase: String(raw.phase || "idle"),
    taskId: optionalString(raw.taskId),
    recordingSessionId: optionalString(raw.recordingSessionId),
    lastTranscriptId: optionalString(raw.lastTranscriptId),
    lastAsrText: String(raw.lastAsrText || ""),
    lastText: String(raw.lastText || ""),
    lastCreatedAtMs: optionalNumber(raw.lastCreatedAtMs),
    diagnosticCode: optionalString(raw.diagnosticCode),
    diagnosticLine: String(raw.diagnosticLine || ""),
    primaryLabel: String(raw.primaryLabel || "START"),
    primaryDisabled: raw.primaryDisabled === true,
    canRewrite: raw.canRewrite === true,
    canInsert: raw.canInsert === true,
    canCopy: raw.canCopy === true,
  };
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
  if (command === "rewriteLast") return "REWRITE FAILED";
  if (command === "insertLast") return "INSERT FAILED";
  if (command === "copyLast") return "COPY FAILED";
  if (command === "cancel") return "CANCEL FAILED";
  return "WORKFLOW COMMAND FAILED";
}
