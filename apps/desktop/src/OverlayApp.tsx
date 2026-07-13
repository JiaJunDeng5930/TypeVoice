import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  appendTranscript,
  textFromTranscriptionPartial,
} from "./domain/overlaySession";
import {
  EMPTY_WORKFLOW_VIEW,
  overlayViewFromWorkflow,
  shouldAcceptWorkflowProjection,
  workflowDisplayText,
  workflowProjectionFromPayload,
  workflowProjectionRevision,
} from "./domain/workflowView";
import { defaultTauriGateway } from "./infra/runtimePorts";
import type {
  OverlayConfig,
  Settings,
  UiEvent,
  WorkflowView,
} from "./types";

const DEFAULT_OVERLAY_CONFIG: OverlayConfig = {
  background_opacity: 0.78,
  font_size_px: 32,
  width_px: 960,
  height_px: 160,
  position_x: null,
  position_y: null,
};

export default function OverlayApp() {
  const [workflow, setWorkflow] = useState<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const workflowRef = useRef<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const latestWorkflowRevisionRef = useRef<number | null>(null);
  const [draftText, setDraftText] = useState("");
  const [liveText, setLiveText] = useState("");
  const [config, setConfig] = useState<OverlayConfig>(DEFAULT_OVERLAY_CONFIG);
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

  const displayText = useMemo(
    () => appendTranscript(draftText, liveText),
    [draftText, liveText],
  );

  const overlayView = useMemo(
    () => overlayViewFromWorkflow(workflow),
    [workflow],
  );

  const subtitleText = displayText.trim() || overlayView.status;

  const acceptWorkflowView = useCallback((next: WorkflowView): boolean => {
    if (!shouldAcceptWorkflowProjection(latestWorkflowRevisionRef.current, next)) return false;
    latestWorkflowRevisionRef.current = workflowProjectionRevision(next);
    workflowRef.current = next;
    setWorkflow(next);

    if (next.mode === "recording") {
      setDraftText("");
      setLiveText("");
      return true;
    }

    const seedText = workflowDisplayText(next).trim();
    if (seedText && next.mode === "ready") {
      setDraftText(seedText);
      setLiveText("");
    } else if (seedText && !draftRef.current.trim() && !liveRef.current.trim()) {
      setDraftText(seedText);
    }
    return true;
  }, []);

  const refreshWorkflowSnapshot = useCallback(async () => {
    const payload = await defaultTauriGateway.invoke<unknown>("workflow_snapshot");
    const next = workflowProjectionFromPayload(payload);
    if (!next) throw new Error("workflow_snapshot returned a malformed projection");
    acceptWorkflowView(next);
  }, [acceptWorkflowView]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      const next = await defaultTauriGateway.invoke<OverlayConfig>("overlay_config");
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
  }, []);

  useEffect(() => {
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
          void defaultTauriGateway.invoke("overlay_save_position");
        }, 180);
      });
    })();
    return () => {
      if (savePositionTimerRef.current !== null) {
        window.clearTimeout(savePositionTimerRef.current);
      }
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const settings = (await defaultTauriGateway.invoke("get_settings")) as Settings;
      if (cancelled) return;
      await defaultTauriGateway.invoke("overlay_set_state", {
        state: {
          visible: settings.hotkeys_show_overlay === true && overlayView.visible,
          status: overlayView.status,
          detail: overlayView.detail,
          ts_ms: Date.now(),
        },
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [overlayView.detail, overlayView.status, overlayView.visible]);

  useEffect(() => {
    let cancelled = false;
    const unlistenFns: Array<() => void> = [];
    const track = (fn: () => void): boolean => {
      if (cancelled) {
        fn();
        return false;
      }
      unlistenFns.push(fn);
      return true;
    };

    void (async () => {
      const stopUiEvents = await defaultTauriGateway.listen<UiEvent>("ui_event", (event) => {
        if (cancelled || !event || event.kind === "audio.level") return;

        if (event.kind === "workflow.state") {
          const next = workflowProjectionFromPayload(event.payload);
          if (next) acceptWorkflowView(next);
          return;
        }

        if (event.kind === "transcription.partial") {
          const activeRunId = workflowRef.current.activeRun?.runId ?? null;
          setLiveText(textFromTranscriptionPartial(event, activeRunId));
          return;
        }

        if (!eventBelongsToCurrentProjection(event, workflowRef.current)) return;

        if (event.status === "failed" || event.status === "cancelled") {
          setLiveText("");
        }
      });
      if (!track(stopUiEvents)) return;

      await refreshWorkflowSnapshot();
    })();

    return () => {
      cancelled = true;
      for (const fn of unlistenFns) fn();
    };
  }, [acceptWorkflowView, refreshWorkflowSnapshot]);

  return (
    <SubtitleOverlay
      config={config}
      text={subtitleText}
      status={overlayView.status}
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
  status: string;
  visible: boolean;
  onDragActivity: (active: boolean) => void;
};

function SubtitleOverlay({
  config,
  text,
  status,
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
      >
        {text}
      </div>
      <div className="srOnly" role="status" aria-live="polite">
        {status}
      </div>
    </div>
  );
}

function eventBelongsToCurrentProjection(event: UiEvent, view: WorkflowView): boolean {
  const runId = optionalString(event.taskId);
  return Boolean(
    runId
    && (view.activeRun?.runId === runId || view.lastRun?.runId === runId),
  );
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}
