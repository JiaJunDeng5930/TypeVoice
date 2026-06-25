import { useCallback, useEffect, useRef, useState } from "react";
import {
  defaultTauriGateway,
} from "../infra/runtimePorts";
import { buildDiagnostic, buildUiEventDiagnostic, userMessageFromDiagnostic } from "../domain/diagnostic";
import {
  EMPTY_WORKFLOW_VIEW,
  primaryActionLabel,
  statusLabelFromPhase,
  workflowPhaseName,
  workflowViewFromPayload,
} from "../domain/workflowView";
import type {
  RuntimeToolchainStatus,
  Settings,
  TranscriptionMetrics,
  UiEvent,
  WorkflowCommand,
  WorkflowView,
} from "../types";
import { IconStart, IconStop, IconTranscribing } from "../ui/icons";

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
};

export function MainScreen({
  settings,
  pushToast,
  onHistoryChanged,
}: Props) {
  const [workflow, setWorkflow] = useState<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const [liveTranscript, setLiveTranscript] = useState("");
  const autoRewriteStartedRef = useRef<Set<string>>(new Set());
  const autoInsertStartedRef = useRef<Set<string>>(new Set());

  const runAutoInsert = useCallback(async (view: WorkflowView) => {
    const phase = workflowPhaseName(view.phase);
    const transcriptId = optionalString(view.lastTranscriptId);
    const text = (view.lastText || view.lastAsrText || "").trim();
    if ((phase !== "transcribed" && phase !== "rewritten") || !transcriptId || !text) return;
    if (autoInsertStartedRef.current.has(transcriptId)) return;
    autoInsertStartedRef.current.add(transcriptId);
    try {
      await defaultTauriGateway.invoke("workflow_insert", { req: { text } });
      const refreshed = await defaultTauriGateway.invoke<WorkflowView>("workflow_snapshot");
      setWorkflow(refreshed);
      onHistoryChanged();
    } catch (err) {
      const diag = buildDiagnostic(err, "Text could not be pasted");
      pushToast(diag.title, "danger");
      try {
        const refreshed = await defaultTauriGateway.invoke<WorkflowView>("workflow_snapshot");
        setWorkflow(refreshed);
      } catch (refreshErr) {
        const refreshDiag = buildDiagnostic(refreshErr, "WORKFLOW STATE FAILED");
        pushToast(refreshDiag.title, "danger");
      }
    }
  }, [onHistoryChanged, pushToast]);

  const runAutoRewrite = useCallback(async (view: WorkflowView) => {
    const phase = workflowPhaseName(view.phase);
    const transcriptId = optionalString(view.lastTranscriptId);
    const text = (view.lastText || view.lastAsrText || "").trim();
    if (phase !== "transcribed" || !transcriptId || !text) return;
    if (autoRewriteStartedRef.current.has(transcriptId)) return;
    if (settings?.rewrite_enabled !== true) return;
    autoRewriteStartedRef.current.add(transcriptId);
    try {
      await defaultTauriGateway.invoke("workflow_rewrite", { req: { text } });
      const refreshed = await defaultTauriGateway.invoke<WorkflowView>("workflow_snapshot");
      setWorkflow(refreshed);
      await runAutoInsert(refreshed);
    } catch (err) {
      const diag = buildDiagnostic(err, "Text improvement failed");
      pushToast(diag.title, "danger");
      try {
        const refreshed = await defaultTauriGateway.invoke<WorkflowView>("workflow_snapshot");
        setWorkflow(refreshed);
      } catch (refreshErr) {
        const refreshDiag = buildDiagnostic(refreshErr, "WORKFLOW STATE FAILED");
        pushToast(refreshDiag.title, "danger");
      }
    }
  }, [pushToast, runAutoInsert, settings?.rewrite_enabled]);

  const acceptWorkflowView = useCallback(async (next: WorkflowView, autoContinue: boolean) => {
    setWorkflow(next);
    const phase = workflowPhaseName(next.phase);
    if (phase === "recording") setLiveTranscript("");
    if (!autoContinue) return;
    if (phase === "transcribed") {
      if (settings?.rewrite_enabled === true) {
        await runAutoRewrite(next);
      } else {
        await runAutoInsert(next);
      }
      return;
    }
    if (phase === "rewritten") {
      await runAutoInsert(next);
    }
  }, [runAutoInsert, runAutoRewrite, settings?.rewrite_enabled]);

  useEffect(() => {
    (async () => {
      const view = await defaultTauriGateway.invoke<WorkflowView>("workflow_snapshot");
      await acceptWorkflowView(view, false);
    })().catch((err) => {
      const diag = buildDiagnostic(err, "WORKFLOW STATE FAILED");
      pushToast(diag.title, "danger");
    });
  }, [acceptWorkflowView, pushToast]);

  useEffect(() => {
    (async () => {
      try {
        const runtime = await defaultTauriGateway.invoke<RuntimeToolchainStatus>("runtime_toolchain_status");
        if (!runtime.ready) pushToast("Local audio tools need repair", "danger");
      } catch {
      }
    })();
  }, [pushToast]);

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
      const unlistenUiEvent = await defaultTauriGateway.listen<UiEvent>("ui_event", async (ev) => {
        if (!ev || ev.kind === "audio.level") return;
        if (ev.kind === "transcription.partial") {
          const partial = transcriptionPartialPayload(ev.payload);
          if (partial?.text) setLiveTranscript(partial.text);
          return;
        }
        if (ev.kind === "workflow.state") {
          const next = workflowViewFromPayload(ev.payload);
          if (next) {
            await acceptWorkflowView(next, true);
          }
          return;
        }
        if (isDisplayFailureEvent(ev.kind, ev.status)) return;
        if (ev.kind === "workflow.task.failed") {
          const diag = buildUiEventDiagnostic(ev, failureTitleFromStage(ev.stage));
          if (isAsrFailureStage(ev.stage)) {
            const transcriptId = optionalString(ev.taskId);
            if (transcriptId) {
              try {
                const next = await defaultTauriGateway.invoke<WorkflowView>("workflow_report_asr_failed", {
                  req: {
                    transcriptId,
                    code: optionalString(ev.errorCode) || "E_TRANSCRIBE_FAILED",
                    message: ev.message,
                  },
                });
                await acceptWorkflowView(next, false);
              } catch (err) {
                const diag = buildDiagnostic(err, "WORKFLOW EVENT FAILED");
                pushToast(diag.title, "danger");
              }
            }
          }
          pushToast(diag.title, "danger");
          return;
        }
        if (ev.status === "failed") {
          const diag = buildUiEventDiagnostic(ev, failureTitleFromStage(ev.stage));
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
              const next = await defaultTauriGateway.invoke<WorkflowView>("workflow_report_asr_empty", {
                req: { transcriptId },
              });
              await acceptWorkflowView(next, false);
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
            const next = await defaultTauriGateway.invoke<WorkflowView>("workflow_report_asr_completed", {
              req: {
                transcriptId: completed.transcriptId,
                text: completed.asrText,
                metrics: completed.metrics,
              },
            });
            await acceptWorkflowView(next, true);
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
  }, [acceptWorkflowView, onHistoryChanged, pushToast]);

  async function sendWorkflowCommand(command: WorkflowCommand) {
    try {
      const next = await defaultTauriGateway.invoke<WorkflowView>("workflow_command", { req: { command } });
      await acceptWorkflowView(next, false);
      if (command === "copyLast") {
        pushToast("Text copied", "ok");
      }
    } catch (err) {
      const diag = buildDiagnostic(err, commandErrorTitle(command));
      pushToast(diag.title, "danger");
      try {
        const refreshed = await defaultTauriGateway.invoke<WorkflowView>("workflow_snapshot");
        await acceptWorkflowView(refreshed, false);
      } catch (refreshErr) {
        const refreshDiag = buildDiagnostic(refreshErr, "WORKFLOW STATE FAILED");
        pushToast(refreshDiag.title, "danger");
      }
    }
  }

  const phase = workflowPhaseName(workflow.phase);
  const hint = primaryActionLabel(workflow.primaryLabel || "START");
  const streamText = phase === "recording" || phase === "transcribing" ? liveTranscript : "";
  const statusLabel = phase === "idle" ? "" : statusLabelFromPhase(phase);
  const resultStatusLabel = statusLabelFromPhase(phase);
  const diagnosticMessage = userMessageFromDiagnostic(workflow.diagnosticCode, workflow.diagnosticLine);

  return (
    <div className="pageSurface mainSurface">
      <div className="voiceDock">
        <button
          type="button"
          className={`mainButton ${phase === "transcribing" ? "isBusy" : ""}`}
          onClick={() => void sendWorkflowCommand("primary")}
          disabled={workflow.primaryDisabled}
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
        </button>

        <div className="mainHint">{statusLabel}</div>
      </div>

      <div className="resultSheet">
        <div className="resultHeader">
          <div className="sectionTitle">current transcript</div>
          <span
            className={`resultStatusIcon status-${phase}`}
            aria-label={resultStatusLabel}
            title={resultStatusLabel}
          />
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
          {diagnosticMessage || ""}
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

function isDisplayFailureEvent(kind: string, status: string | null | undefined): boolean {
  return kind === "transcription.stage" && status === "failed";
}

function isAsrFailureStage(stage: string | null | undefined): boolean {
  return stage === "Record" || stage === "Transcribe";
}

function failureTitleFromStage(stage: string | null | undefined): string {
  if (stage === "Rewrite") return "Text improvement failed";
  if (stage === "Insert") return "Text could not be pasted";
  return "Speech recognition failed";
}

function commandErrorTitle(command: WorkflowCommand): string {
  if (command === "rewriteLast") return "Text improvement failed";
  if (command === "insertLast") return "Text could not be pasted";
  if (command === "copyLast") return "Copy failed";
  if (command === "cancel") return "Cancel failed";
  return "Recording action failed";
}
