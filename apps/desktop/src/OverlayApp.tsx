import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createBackendClient } from "./infra/backendClient";
import { defaultTauriGateway } from "./infra/runtimePorts";
import {
  appendTranscript,
  textFromRewriteCompleted,
  textFromTranscriptionCompleted,
  textFromTranscriptionPartial,
} from "./domain/overlaySession";
import {
  canTogglePrimaryFromOverlay,
  EMPTY_WORKFLOW_VIEW,
  overlayViewFromWorkflow,
  workflowPhaseName,
  workflowViewFromPayload,
} from "./domain/workflowView";
import type { OverlayConfig, Settings, WorkflowView } from "./types";

type GlobalHotkeyEvent = {
  action: "primary";
  tsMs?: number;
};

const DEFAULT_OVERLAY_CONFIG: OverlayConfig = {
  background_opacity: 0.78,
  font_size_px: 32,
  width_px: 960,
  height_px: 160,
  position_x: null,
  position_y: null,
};

export default function OverlayApp() {
  const client = useMemo(() => createBackendClient(defaultTauriGateway), []);
  const [workflow, setWorkflow] = useState<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const [draftText, setDraftText] = useState("");
  const [liveText, setLiveText] = useState("");
  const [config, setConfig] = useState<OverlayConfig>(DEFAULT_OVERLAY_CONFIG);
  const phaseRef = useRef("idle");
  const draftRef = useRef("");
  const liveRef = useRef("");
  const dragActiveRef = useRef(false);
  const savePositionTimerRef = useRef<number | null>(null);

  useEffect(() => {
    document.body.classList.add("isOverlay");
    return () => document.body.classList.remove("isOverlay");
  }, []);

  useEffect(() => {
    draftRef.current = draftText;
  }, [draftText]);

  useEffect(() => {
    liveRef.current = liveText;
  }, [liveText]);

  useEffect(() => {
    phaseRef.current = workflowPhaseName(workflow.phase);
  }, [workflow]);

  const displayText = useMemo(
    () => appendTranscript(draftText, liveText),
    [draftText, liveText],
  );

  const overlayView = useMemo(
    () => overlayViewFromWorkflow(workflow),
    [workflow],
  );

  const subtitleText = displayText.trim() || overlayView.status;

  const acceptWorkflowView = useCallback((next: WorkflowView) => {
    const phase = workflowPhaseName(next.phase);
    const seedText = (next.lastText || next.lastAsrText).trim();
    setWorkflow(next);
    if (phase === "recording") {
      setDraftText("");
      setLiveText("");
    }
    if (
      (phase === "transcribed" || phase === "rewritten")
      && seedText
      && !draftRef.current.trim()
      && !liveRef.current.trim()
    ) {
      setDraftText(next.lastText || next.lastAsrText);
      setLiveText("");
    }
  }, []);

  const refreshWorkflowSnapshot = useCallback(async () => {
    acceptWorkflowView(await client.workflowSnapshot());
  }, [acceptWorkflowView, client]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      const next = await client.overlayConfig();
      if (!cancelled) setConfig(next);
      const stop = await defaultTauriGateway.listen<OverlayConfig>(
        "tv_overlay_config_changed",
        (updated) => {
          if (!cancelled) setConfig(updated);
        },
      );
      if (cancelled) {
        stop();
      } else {
        unlisten = stop;
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [client]);

  useEffect(() => {
    if (!hasTauriRuntime()) return;
    let unlisten: (() => void) | null = null;
    const currentWindow = getCurrentWindow();
    void (async () => {
      unlisten = await currentWindow.onMoved(() => {
        if (!dragActiveRef.current) return;
        if (savePositionTimerRef.current !== null) {
          window.clearTimeout(savePositionTimerRef.current);
        }
        savePositionTimerRef.current = window.setTimeout(() => {
          savePositionTimerRef.current = null;
          void client.overlaySavePosition();
        }, 180);
      });
    })();
    return () => {
      if (savePositionTimerRef.current !== null) {
        window.clearTimeout(savePositionTimerRef.current);
      }
      unlisten?.();
    };
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const settings = (await defaultTauriGateway.invoke("get_settings")) as Settings;
      if (cancelled) return;
      await client.overlaySetState({
        visible: settings.hotkeys_show_overlay === true && overlayView.visible,
        status: overlayView.status,
        detail: overlayView.detail,
        ts_ms: Date.now(),
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [client, overlayView.detail, overlayView.status, overlayView.visible]);

  const runPrimaryFromAlt = useCallback(async () => {
    const phase = phaseRef.current;
    if (!canTogglePrimaryFromOverlay(phase)) return;

    if (phase !== "recording") {
      setLiveText("");
    }

    try {
      acceptWorkflowView(await client.workflowCommand({ command: "primary" }));
    } catch {
      await refreshWorkflowSnapshot();
    }
  }, [acceptWorkflowView, client, refreshWorkflowSnapshot]);

  useEffect(() => {
    void refreshWorkflowSnapshot();
  }, [refreshWorkflowSnapshot]);

  useEffect(() => {
    let cancelled = false;
    const unlistenFns: Array<() => void> = [];
    const track = (fn: () => void) => {
      if (cancelled) {
        fn();
      } else {
        unlistenFns.push(fn);
      }
    };

    (async () => {
      track(await defaultTauriGateway.listen<GlobalHotkeyEvent>("tv_global_hotkey", async (event) => {
        if (!event) return;
        if (event.action === "primary") {
          await runPrimaryFromAlt();
        }
      }));

      track(await client.listenUiEvent(async (event) => {
        if (!event || event.kind === "audio.level") return;

        if (event.kind === "workflow.state") {
          const next = workflowViewFromPayload(event.payload);
          if (next) acceptWorkflowView(next);
          return;
        }

        if (event.kind === "transcription.partial") {
          setLiveText(textFromTranscriptionPartial(event));
          return;
        }

        if (event.kind === "transcription.completed") {
          const result = textFromTranscriptionCompleted(event);
          if (result.transcriptId && result.asrText.trim() && result.metrics) {
            const next = await client.workflowReportAsrCompleted({
              transcriptId: result.transcriptId,
              text: result.asrText,
              metrics: result.metrics,
            });
            acceptWorkflowView(next);
          } else {
            await refreshWorkflowSnapshot();
          }
          setDraftText((prev) => appendTranscript(prev, result.asrText));
          setLiveText("");
          return;
        }

        if (event.kind === "transcription.empty") {
          const transcriptId = optionalString(event.taskId);
          if (transcriptId) {
            acceptWorkflowView(await client.workflowReportAsrEmpty({ transcriptId }));
          } else {
            await refreshWorkflowSnapshot();
          }
          setLiveText("");
          return;
        }

        if (event.kind === "rewrite.completed") {
          const result = textFromRewriteCompleted(event);
          if (result.finalText.trim()) setDraftText(result.finalText);
          setLiveText("");
          return;
        }

        if (isDisplayFailureEvent(event.kind, event.status)) return;
        if (event.kind === "workflow.task.failed" && isAsrFailureStage(event.stage)) {
          const transcriptId = optionalString(event.taskId);
          if (transcriptId) {
            acceptWorkflowView(await client.workflowReportAsrFailed({
              transcriptId,
              code: optionalString(event.errorCode) || "E_TRANSCRIBE_FAILED",
              message: event.message,
            }));
          } else {
            await refreshWorkflowSnapshot();
          }
          return;
        }

        if (event.status === "failed") {
          await refreshWorkflowSnapshot();
        }
      }));
    })();

    return () => {
      cancelled = true;
      for (const fn of unlistenFns) fn();
    };
  }, [acceptWorkflowView, client, refreshWorkflowSnapshot, runPrimaryFromAlt]);

  return (
    <SubtitleOverlay
      config={config}
      text={subtitleText}
      visible={overlayView.visible}
      onDragActivity={(active) => {
        dragActiveRef.current = active;
      }}
    />
  );
}

type SubtitleOverlayProps = {
  config: OverlayConfig;
  text: string;
  visible: boolean;
  onDragActivity: (active: boolean) => void;
};

function SubtitleOverlay({
  config,
  text,
  visible,
  onDragActivity,
}: SubtitleOverlayProps) {
  const style = {
    "--subtitle-bg-opacity": String(config.background_opacity),
    "--subtitle-font-size": `${config.font_size_px}px`,
  } as CSSProperties;

  return (
    <div
      className={`subtitleOverlayRoot ${visible ? "" : "isHidden"}`}
      data-tauri-drag-region
      style={style}
      onPointerDown={() => onDragActivity(true)}
      onPointerUp={() => onDragActivity(false)}
      onPointerCancel={() => onDragActivity(false)}
      onPointerLeave={(event) => {
        if (event.buttons === 0) onDragActivity(false);
      }}
    >
      <div
        className="subtitleOverlayText"
        data-tauri-drag-region
        role="status"
        aria-live="polite"
      >
        {text}
      </div>
    </div>
  );
}

function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function isDisplayFailureEvent(kind: string, status: string | null | undefined): boolean {
  return kind === "transcription.stage" && status === "failed";
}

function isAsrFailureStage(stage: string | null | undefined): boolean {
  return stage === "Record" || stage === "Transcribe";
}
