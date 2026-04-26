import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createBackendClient } from "./infra/backendClient";
import { defaultTauriGateway } from "./infra/runtimePorts";
import {
  appendTranscript,
  overlayKeyAction,
  textFromRewriteCompleted,
  textFromTranscriptionCompleted,
  textFromTranscriptionPartial,
} from "./domain/overlaySession";
import type { WorkflowView } from "./types";

type OverlayState = {
  visible: boolean;
  status: string;
  detail?: string | null;
  ts_ms: number;
};

type GlobalHotkeyEvent = {
  action: "altTap" | "insertOverlay";
  tsMs?: number;
};

const OVERLAY_WIDTH = 420;
const MIN_OVERLAY_HEIGHT = 112;
const MAX_OVERLAY_HEIGHT = 520;

function isActivePhase(phase: string): boolean {
  return ["recording", "transcribing", "rewriting", "inserting"].includes(phase);
}

function canAltToggle(phase: string): boolean {
  return ["idle", "recording", "cancelled", "failed"].includes(phase);
}

function statusFromPhase(phase: string): string {
  if (phase === "recording") return "REC";
  if (phase === "transcribing") return "TRANSCRIBING";
  if (phase === "rewriting") return "REWRITING";
  if (phase === "rewritten") return "REWRITTEN";
  if (phase === "inserting") return "INSERTING";
  if (phase === "transcribed") return "TRANSCRIBED";
  if (phase === "failed") return "ERROR";
  return "READY";
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
    lastCreatedAtMs: typeof raw.lastCreatedAtMs === "number" ? raw.lastCreatedAtMs : null,
    diagnosticCode: optionalString(raw.diagnosticCode),
    diagnosticLine: String(raw.diagnosticLine || ""),
    primaryLabel: String(raw.primaryLabel || "START"),
    primaryDisabled: raw.primaryDisabled === true,
    canRewrite: raw.canRewrite === true,
    canInsert: raw.canInsert === true,
    canCopy: raw.canCopy === true,
  };
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

export default function OverlayApp() {
  const client = useMemo(() => createBackendClient(defaultTauriGateway), []);
  const [visible, setVisible] = useState(false);
  const [status, setStatus] = useState("READY");
  const [draftText, setDraftText] = useState("");
  const [liveText, setLiveText] = useState("");
  const [detail, setDetail] = useState<string | null>(null);
  const [workflow, setWorkflow] = useState<WorkflowView | null>(null);
  const [transcriptId, setTranscriptId] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<"rewrite" | "insert" | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const phaseRef = useRef("idle");
  const draftRef = useRef("");
  const liveRef = useRef("");
  const transcriptIdRef = useRef<string | null>(null);
  const busyRef = useRef<"rewrite" | "insert" | null>(null);

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
    transcriptIdRef.current = transcriptId;
  }, [transcriptId]);

  useEffect(() => {
    busyRef.current = busyAction;
  }, [busyAction]);

  useEffect(() => {
    phaseRef.current = String(workflow?.phase || "idle").toLowerCase();
  }, [workflow]);

  const resizeOverlay = useCallback(() => {
    const root = rootRef.current;
    if (!root || !visible) return;
    const measured = Math.ceil(root.scrollHeight + 2);
    const height = Math.max(MIN_OVERLAY_HEIGHT, Math.min(MAX_OVERLAY_HEIGHT, measured));
    void client.overlayResize({ width: OVERLAY_WIDTH, height });
  }, [client, visible]);

  useEffect(() => {
    resizeOverlay();
  }, [draftText, liveText, detail, status, resizeOverlay]);

  useEffect(() => {
    if (!visible) return;
    void client.overlaySetState({
      visible: true,
      status,
      detail,
      ts_ms: Date.now(),
    });
  }, [client, detail, status, visible]);

  const hideOverlay = useCallback(async () => {
    setVisible(false);
    setStatus("READY");
    setDetail(null);
    await client.overlaySetState({
      visible: false,
      status: "READY",
      detail: null,
      ts_ms: Date.now(),
    });
  }, [client]);

  const showOverlay = useCallback((nextStatus: string) => {
    setVisible(true);
    setStatus(nextStatus);
    setDetail(null);
  }, []);

  const runPrimaryFromAlt = useCallback(async () => {
    const phase = phaseRef.current;
    if (!canAltToggle(phase) || busyRef.current) return;

    showOverlay(phase === "recording" ? "TRANSCRIBING" : "REC");
    if (phase !== "recording") {
      setLiveText("");
    }

    try {
      const next = await client.workflowCommand({ command: "primary" });
      setWorkflow(next);
      setStatus(statusFromPhase(String(next.phase || "idle").toLowerCase()));
    } catch (err) {
      setStatus("ERROR");
      setDetail(String(err));
    }
  }, [client, showOverlay]);

  const runRewrite = useCallback(async () => {
    if (busyRef.current) return;
    const id = transcriptIdRef.current;
    const text = draftRef.current.trim();
    if (!id || !text || isActivePhase(phaseRef.current)) return;

    setBusyAction("rewrite");
    setStatus("REWRITING");
    setDetail(null);
    try {
      const result = await client.workflowRewrite({
        transcriptId: id,
        text,
      });
      setDraftText(result.finalText);
      setLiveText("");
      setTranscriptId(result.transcriptId);
      setStatus("REWRITTEN");
    } catch (err) {
      setStatus("ERROR");
      setDetail(String(err));
    } finally {
      setBusyAction(null);
    }
  }, [client]);

  const runInsert = useCallback(async () => {
    if (busyRef.current) return;
    const text = appendTranscript(draftRef.current, liveRef.current).trim();
    if (!text) return;

    setBusyAction("insert");
    setStatus("INSERTING");
    setDetail(null);
    setVisible(false);
    await client.overlaySetState({
      visible: false,
      status: "INSERTING",
      detail: null,
      ts_ms: Date.now(),
    });

    try {
      const id = transcriptIdRef.current;
      if (!id) throw new Error("E_INSERT_TRANSCRIPT_ID_MISSING: transcript_id is required");
      await client.workflowInsert({
        transcriptId: id,
        text,
      });
      setDraftText("");
      setLiveText("");
      setTranscriptId(null);
      await hideOverlay();
    } catch (err) {
      setVisible(true);
      setStatus("ERROR");
      setDetail(String(err));
    } finally {
      setBusyAction(null);
    }
  }, [client, hideOverlay]);

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
        if (event.action === "altTap") {
          await runPrimaryFromAlt();
          return;
        }
        if (event.action === "insertOverlay") {
          await runInsert();
        }
      }));

      track(await defaultTauriGateway.listen<OverlayState>("tv_overlay_state", (state) => {
        if (!state) return;
        if (!state.visible && !draftRef.current.trim() && !liveRef.current.trim()) {
          setVisible(false);
        }
        if (state.status) setStatus(String(state.status).toUpperCase());
        setDetail(state.detail || null);
      }));

      track(await client.listenUiEvent((event) => {
        if (!event || event.kind === "audio.level") return;

        if (event.kind === "workflow.state") {
          const next = workflowViewFromPayload(event.payload);
          if (!next) return;
          const phase = String(next.phase || "idle").toLowerCase();
          setWorkflow(next);
          setStatus(statusFromPhase(phase));
          if (phase === "recording" || phase === "transcribing" || phase === "rewriting" || phase === "rewritten" || phase === "transcribed") {
            setVisible(true);
          }
          return;
        }

        if (event.kind === "transcription.partial") {
          const text = textFromTranscriptionPartial(event);
          setLiveText(text);
          setVisible(true);
          setStatus("REC");
          return;
        }

        if (event.kind === "transcription.completed") {
          const result = textFromTranscriptionCompleted(event);
          setDraftText((prev) => appendTranscript(prev, result.asrText));
          setLiveText("");
          if (result.transcriptId) setTranscriptId(result.transcriptId);
          setVisible(true);
          setStatus("TRANSCRIBED");
          return;
        }

        if (event.kind === "rewrite.completed") {
          const result = textFromRewriteCompleted(event);
          if (result.finalText.trim()) setDraftText(result.finalText);
          setLiveText("");
          if (result.transcriptId) setTranscriptId(result.transcriptId);
          setVisible(true);
          setStatus("REWRITTEN");
          return;
        }

        if (event.status === "failed") {
          setVisible(true);
          setStatus("ERROR");
          setDetail(event.errorCode || event.message);
        }
      }));
    })().catch((err) => {
      setVisible(true);
      setStatus("ERROR");
      setDetail(String(err));
    });

    return () => {
      cancelled = true;
      for (const fn of unlistenFns) fn();
    };
  }, [client, runInsert, runPrimaryFromAlt]);

  const displayText = useMemo(
    () => appendTranscript(draftText, liveText),
    [draftText, liveText],
  );
  const tone = status === "ERROR" ? "danger" : status === "REWRITTEN" || status === "TRANSCRIBED" ? "ok" : "default";

  if (!visible) {
    return <div ref={rootRef} className="transcriptOverlayRoot isHidden" />;
  }

  return (
    <div ref={rootRef} className={`transcriptOverlayRoot tone-${tone}`}>
      <div className="transcriptOverlayTop">
        <div className="transcriptOverlayBadge">{busyAction ? status : status}</div>
        {detail ? <div className="transcriptOverlayDetail">{detail}</div> : null}
      </div>
      <textarea
        className="transcriptOverlayEditor"
        value={displayText}
        spellCheck={false}
        onChange={(event) => {
          setDraftText(event.currentTarget.value);
          setLiveText("");
        }}
        onKeyDown={(event) => {
          const action = overlayKeyAction(event);
          if (action === "none" || action === "newline") return;
          event.preventDefault();
          if (action === "rewrite") {
            void runRewrite();
            return;
          }
          void runInsert();
        }}
      />
    </div>
  );
}
