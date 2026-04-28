import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import type { WorkflowView } from "./types";

type GlobalHotkeyEvent = {
  action: "primary";
  tsMs?: number;
};

const OVERLAY_WIDTH = 420;
const MIN_OVERLAY_HEIGHT = 112;
const MAX_OVERLAY_HEIGHT = 520;

export default function OverlayApp() {
  const client = useMemo(() => createBackendClient(defaultTauriGateway), []);
  const [workflow, setWorkflow] = useState<WorkflowView>(EMPTY_WORKFLOW_VIEW);
  const [draftText, setDraftText] = useState("");
  const [liveText, setLiveText] = useState("");
  const rootRef = useRef<HTMLDivElement | null>(null);
  const phaseRef = useRef("idle");
  const draftRef = useRef("");
  const liveRef = useRef("");

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

  const acceptWorkflowView = useCallback((next: WorkflowView) => {
    const phase = workflowPhaseName(next.phase);
    const seedText = (next.lastText || next.lastAsrText).trim();
    setWorkflow(next);
    if (phase === "recording") {
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

  const resizeOverlay = useCallback(() => {
    const root = rootRef.current;
    if (!root || !overlayView.visible) return;
    const measured = Math.ceil(root.scrollHeight + 2);
    const height = Math.max(MIN_OVERLAY_HEIGHT, Math.min(MAX_OVERLAY_HEIGHT, measured));
    void client.overlayResize({ width: OVERLAY_WIDTH, height });
  }, [client, overlayView.visible]);

  useEffect(() => {
    resizeOverlay();
  }, [displayText, overlayView.detail, overlayView.status, resizeOverlay]);

  useEffect(() => {
    void client.overlaySetState({
      visible: overlayView.visible,
      status: overlayView.status,
      detail: overlayView.detail,
      ts_ms: Date.now(),
    });
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
          setDraftText((prev) => appendTranscript(prev, result.asrText));
          setLiveText("");
          return;
        }

        if (event.kind === "rewrite.completed") {
          const result = textFromRewriteCompleted(event);
          if (result.finalText.trim()) setDraftText(result.finalText);
          setLiveText("");
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

  if (!overlayView.visible) {
    return <div ref={rootRef} className="transcriptOverlayRoot isHidden" />;
  }

  return (
    <div ref={rootRef} className={`transcriptOverlayRoot tone-${overlayView.tone}`}>
      <div className="transcriptOverlayTop">
        <div className="transcriptOverlayBadge">{overlayView.status}</div>
        {overlayView.detail ? <div className="transcriptOverlayDetail">{overlayView.detail}</div> : null}
      </div>
      <textarea
        className="transcriptOverlayEditor"
        value={displayText}
        spellCheck={false}
        onChange={(event) => {
          setDraftText(event.currentTarget.value);
          setLiveText("");
        }}
      />
    </div>
  );
}
