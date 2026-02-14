import { useEffect, useMemo, useRef, useState } from "react";
import {
  browserClipboard,
  browserTimer,
  defaultTauriGateway,
  type ClipboardPort,
  type TauriGateway,
  type TimerPort,
} from "../infra/runtimePorts";
import {
  buildDiagnostic,
  buildTaskEventDiagnostic,
  compactDetail,
  hotkeyCaptureHint,
  toDiagnosticLine,
} from "../domain/diagnostic";
import type {
  HistoryItem,
  RuntimePythonStatus,
  RuntimeToolchainStatus,
  Settings,
  TaskDone,
  TaskEvent,
} from "../types";
import { IconStart, IconStop, IconTranscribing } from "../ui/icons";

type UiState = "idle" | "recording" | "transcribing" | "cancelling";

type HotkeyRecordEvent = {
  kind: "ptt" | "toggle";
  state: "Pressed" | "Released";
  shortcut: string;
  ts_ms: number;
  recording_session_id?: string | null;
  capture_status?: "ok" | "err" | null;
  capture_error_code?: string | null;
  capture_error_message?: string | null;
};

type StopBackendRecordingResult = {
  recordingId: string;
  recordingAssetId: string;
  ext: string;
};

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
  gateway?: TauriGateway;
  timer?: TimerPort;
  clipboard?: ClipboardPort;
};

export function MainScreen({
  settings,
  pushToast,
  onHistoryChanged,
  gateway = defaultTauriGateway,
  timer = browserTimer,
  clipboard = browserClipboard,
}: Props) {
  const [ui, setUi] = useState<UiState>("idle");
  const [hover, setHover] = useState(false);
  const [diagnosticLine, setDiagnosticLine] = useState<string>("");

  const [lastText, setLastText] = useState<string>("");
  const [lastMeta, setLastMeta] = useState<string>("NO LAST RESULT");
  const [lastHover, setLastHover] = useState(false);

  const uiRef = useRef<UiState>("idle");
  useEffect(() => {
    uiRef.current = ui;
  }, [ui]);

  const activeTaskIdRef = useRef<string>("");
  const backendRecordingIdRef = useRef<string>("");
  const hotkeySessionRef = useRef<boolean>(false);
  const pendingRecordingSessionIdRef = useRef<string | null>(null);

  const hasHotkeyConfig =
    typeof settings?.hotkeys_enabled === "boolean" &&
    typeof settings?.hotkeys_show_overlay === "boolean";
  const hotkeysEnabledRef = useRef<boolean>(false);
  const showOverlayRef = useRef<boolean>(false);
  const hasHotkeyConfigRef = useRef<boolean>(hasHotkeyConfig);
  const pushToastRef = useRef(pushToast);
  const onHistoryChangedRef = useRef(onHistoryChanged);

  useEffect(() => {
    hasHotkeyConfigRef.current = hasHotkeyConfig;
    hotkeysEnabledRef.current = hasHotkeyConfig ? settings?.hotkeys_enabled === true : false;
    showOverlayRef.current = hasHotkeyConfig ? settings?.hotkeys_show_overlay === true : false;
    pushToastRef.current = pushToast;
    onHistoryChangedRef.current = onHistoryChanged;
  }, [hasHotkeyConfig, onHistoryChanged, pushToast, settings]);

  const hint = useMemo(() => {
    if (ui === "idle") return "START";
    if (ui === "recording") return "STOP";
    if (ui === "cancelling") return "CANCELLING";
    return "CANCEL";
  }, [ui]);

  async function overlaySet(visible: boolean, status: string, detail?: string | null) {
    if (!showOverlayRef.current) return;
    try {
      await gateway.invoke("overlay_set_state", {
        state: { visible, status, detail: detail || null, ts_ms: Date.now() },
      });
    } catch {
      // ignore
    }
  }

  function overlayFlash(status: string, ms: number, detail?: string | null) {
    void overlaySet(true, status, detail || null);
    timer.setTimeout(() => {
      void overlaySet(false, "IDLE");
    }, ms);
  }

  async function abortRecordingSessionBestEffort(sessionId: string | null) {
    if (!sessionId || !sessionId.trim()) return;
    try {
      await gateway.invoke("abort_recording_session", { recordingSessionId: sessionId });
    } catch {
      // ignore
    }
  }

  useEffect(() => {
    if (!hasHotkeyConfig) {
      pushToast("SETTINGS INVALID: HOTKEY FLAGS MISSING", "danger");
    }
  }, [hasHotkeyConfig, pushToast]);

  useEffect(() => {
    (async () => {
      try {
        const runtime = (await gateway.invoke("runtime_toolchain_status")) as RuntimeToolchainStatus;
        if (!runtime.ready) {
          pushToast("TOOLCHAIN NOT READY", "danger");
        }
      } catch {
        // ignore
      }
      try {
        const runtime = (await gateway.invoke("runtime_python_status")) as RuntimePythonStatus;
        if (!runtime.ready) {
          pushToast("PYTHON NOT READY", "danger");
        }
      } catch {
        // ignore
      }

      try {
        const rows = (await gateway.invoke("history_list", {
          limit: 1,
          beforeMs: null,
        })) as HistoryItem[];
        const h = rows[0];
        if (!h) return;
        const text = h.final_text || h.asr_text || "";
        setLastText(text);
        setLastMeta(new Date(h.created_at_ms).toLocaleString());
      } catch {
        // ignore
      }
    })();
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unlistenFns: Array<() => void> = [];
    const trackUnlisten = (fn: () => void) => {
      if (cancelled) {
        try {
          fn();
        } catch {
          // ignore
        }
        return;
      }
      unlistenFns.push(fn);
    };

    (async () => {
      const unlistenDone = await gateway.listen<TaskDone>("task_done", async (done) => {
        if (!done) return;
        if (done.task_id !== activeTaskIdRef.current) return;
        activeTaskIdRef.current = "";
        setUi("idle");
        setDiagnosticLine("");

        const text = done.final_text || done.asr_text || "";
        setLastText(text);
        setLastMeta(new Date().toLocaleString());
        try {
          await clipboard.copyText(text);
          pushToastRef.current("COPIED", "ok");
        } catch {
          pushToastRef.current("COPY FAILED", "danger");
        }
        if (hotkeySessionRef.current) {
          overlayFlash("COPIED", 800);
          hotkeySessionRef.current = false;
        }
        onHistoryChangedRef.current();
      });
      trackUnlisten(unlistenDone);

      const unlistenEvent = await gateway.listen<TaskEvent>("task_event", (ev) => {
        if (!ev) return;
        if (ev.task_id !== activeTaskIdRef.current) return;

        if (ev.status === "failed" && ev.stage !== "Rewrite") {
          activeTaskIdRef.current = "";
          setUi("idle");
          const diag = buildTaskEventDiagnostic(ev, "TRANSCRIBE FAILED");
          pushToastRef.current(diag.title, "danger");
          setDiagnosticLine(toDiagnosticLine(diag));
          if (hotkeySessionRef.current) {
            overlayFlash("ERROR", 1400, diag.code);
            hotkeySessionRef.current = false;
          }
        }
        if (ev.status === "failed" && ev.stage === "Rewrite") {
          const diag = buildTaskEventDiagnostic(ev, "REWRITE FAILED");
          pushToastRef.current(diag.title, "danger");
          setDiagnosticLine(toDiagnosticLine(diag));
        }
        if (ev.status === "cancelled") {
          activeTaskIdRef.current = "";
          setUi("idle");
          pushToastRef.current("CANCELLED", "default");
          setDiagnosticLine("");
          if (hotkeySessionRef.current) {
            overlayFlash("CANCELLED", 800);
            hotkeySessionRef.current = false;
          }
        }
      });
      trackUnlisten(unlistenEvent);

      const unlistenHotkey = await gateway.listen<HotkeyRecordEvent>("tv_hotkey_record", (hk) => {
        if (!hasHotkeyConfigRef.current) {
          pushToastRef.current("SETTINGS INVALID", "danger");
          return;
        }
        if (!hotkeysEnabledRef.current) return;
        if (!hk) return;

        const cur = uiRef.current;
        if (hk.kind === "ptt") {
          if (hk.state === "Pressed" && cur === "idle") {
            if (hk.capture_status !== "ok" || !hk.recording_session_id) {
              const hint = hotkeyCaptureHint(hk.capture_error_code);
              const detail = compactDetail(
                [hk.capture_error_code || "E_HOTKEY_CAPTURE", hk.capture_error_message || hint]
                  .filter(Boolean)
                  .join(": "),
              );
              setDiagnosticLine(detail);
              pushToastRef.current(hint, "danger");
              void overlaySet(true, "ERROR", hint);
              timer.setTimeout(() => {
                void overlaySet(false, "IDLE");
              }, 1200);
              return;
            }
            void startRecording("hotkey", hk.recording_session_id);
          }
          if (hk.state === "Released" && cur === "recording") {
            void stopAndTranscribe();
          }
          return;
        }

        // toggle
        if (hk.state !== "Pressed") return;
        if (cur === "idle") {
          if (hk.capture_status !== "ok" || !hk.recording_session_id) {
            const hint = hotkeyCaptureHint(hk.capture_error_code);
            const detail = compactDetail(
              [hk.capture_error_code || "E_HOTKEY_CAPTURE", hk.capture_error_message || hint]
                .filter(Boolean)
                .join(": "),
            );
            setDiagnosticLine(detail);
            pushToastRef.current(hint, "danger");
            void overlaySet(true, "ERROR", hint);
            timer.setTimeout(() => {
              void overlaySet(false, "IDLE");
            }, 1200);
            return;
          }
          void startRecording("hotkey", hk.recording_session_id);
        }
        else if (cur === "recording") void stopAndTranscribe();
        else if (cur === "transcribing") void cancelActiveTask();
      });
      trackUnlisten(unlistenHotkey);
    })().catch(() => {
      // ignore
    });
    return () => {
      cancelled = true;
      const staleRecordingId = backendRecordingIdRef.current;
      backendRecordingIdRef.current = "";
      if (staleRecordingId) {
        void gateway.invoke("abort_backend_recording", { recordingId: staleRecordingId }).catch(() => {});
      }
      const staleSessionId = pendingRecordingSessionIdRef.current;
      pendingRecordingSessionIdRef.current = null;
      void abortRecordingSessionBestEffort(staleSessionId);
      for (const fn of unlistenFns) {
        try {
          fn();
        } catch {
          // ignore
        }
      }
    };
  }, []);

  async function startRecording(source: "ui" | "hotkey" = "ui", recordingSessionId: string | null = null) {
    hotkeySessionRef.current = source === "hotkey";
    pendingRecordingSessionIdRef.current = source === "hotkey" ? recordingSessionId : null;
    setDiagnosticLine("");
    if (hotkeySessionRef.current) void overlaySet(true, "REC");
    try {
      const rid = (await gateway.invoke("start_backend_recording")) as string;
      backendRecordingIdRef.current = rid;
      setUi("recording");
    } catch (err) {
      const staleSessionId = pendingRecordingSessionIdRef.current;
      void abortRecordingSessionBestEffort(staleSessionId);
      setUi("idle");
      const diag = buildDiagnostic(err, "RECORDING FAILED");
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      pendingRecordingSessionIdRef.current = null;
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function stopAndTranscribe() {
    const rid = backendRecordingIdRef.current;
    if (!rid) return;
    setUi("transcribing");
    let stopResult: StopBackendRecordingResult | null = null;
    try {
      if (hotkeySessionRef.current) void overlaySet(true, "TRANSCRIBING");
      stopResult = (await gateway.invoke("stop_backend_recording", {
        recordingId: rid,
      })) as StopBackendRecordingResult;
    } catch (err) {
      void gateway.invoke("abort_backend_recording", { recordingId: rid }).catch(() => {});
      backendRecordingIdRef.current = "";
      const staleSessionId = pendingRecordingSessionIdRef.current;
      void abortRecordingSessionBestEffort(staleSessionId);
      setUi("idle");
      pendingRecordingSessionIdRef.current = null;
      const diag = buildDiagnostic(err, "RECORDING FAILED");
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
      return;
    }

    if (!stopResult) return;
    backendRecordingIdRef.current = "";
    try {
      const id = (await gateway.invoke("start_task", {
        req: {
          triggerSource: hotkeySessionRef.current ? "hotkey" : "ui",
          recordMode: "recording_asset",
          recordingAssetId: stopResult.recordingAssetId,
          recordingSessionId: pendingRecordingSessionIdRef.current,
        },
      })) as string;
      activeTaskIdRef.current = id;
      pendingRecordingSessionIdRef.current = null;
    } catch (err) {
      const staleSessionId = pendingRecordingSessionIdRef.current;
      void abortRecordingSessionBestEffort(staleSessionId);
      setUi("idle");
      pendingRecordingSessionIdRef.current = null;
      const diag = buildDiagnostic(err, "TRANSCRIBE FAILED");
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function cancelActiveTask() {
    const id = activeTaskIdRef.current;
    if (!id) return;
    setUi("cancelling");
    try {
      await gateway.invoke("cancel_task", { taskId: id });
      pushToastRef.current("CANCELLING...", "default");
      setDiagnosticLine("");
      if (hotkeySessionRef.current) void overlaySet(true, "CANCELLING");
    } catch (err) {
      setUi("transcribing");
      const diag = buildDiagnostic(err, "CANCEL FAILED");
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function onMainButtonClick() {
    if (ui === "idle") return startRecording("ui");
    if (ui === "recording") return stopAndTranscribe();
    if (ui === "transcribing") return cancelActiveTask();
  }

  async function copyLast() {
    if (!lastText.trim()) return;
    try {
      await clipboard.copyText(lastText);
      pushToastRef.current("COPIED", "ok");
    } catch {
      pushToastRef.current("COPY FAILED", "danger");
    }
  }

  return (
    <div className="card mainCard">
      <div className="mainCenter">
        <button
          type="button"
          className={`mainButton ${ui === "transcribing" || ui === "cancelling" ? "isBusy" : ""}`}
          onClick={onMainButtonClick}
          disabled={ui === "cancelling"}
          onMouseEnter={() => setHover(true)}
          onMouseLeave={() => setHover(false)}
          onFocus={() => setHover(true)}
          onBlur={() => setHover(false)}
          aria-label={hint}
          title={hint}
        >
          {ui === "idle" ? (
            <IconStart size={84} tone="accent" />
          ) : ui === "recording" ? (
            <IconStop size={84} tone="accent" />
          ) : ui === "cancelling" ? (
            <IconStop size={84} tone="accent" />
          ) : (
            <IconTranscribing size={84} tone="accent" />
          )}
        </button>

        <div className="mainHint" aria-hidden={!hover && ui !== "transcribing"}>
          {hover || ui === "transcribing" ? hint : ""}
        </div>
        <div
          className={`mainDiag ${diagnosticLine ? "isVisible" : ""}`}
          aria-hidden={!diagnosticLine}
        >
          {diagnosticLine || ""}
        </div>

        <button
          type="button"
          className="lastLine"
          onClick={copyLast}
          disabled={!lastText.trim()}
          onMouseEnter={() => setLastHover(true)}
          onMouseLeave={() => setLastHover(false)}
          title={lastText.trim() ? "COPY" : ""}
        >
          <span className="lastMeta">{lastMeta}</span>
          <span className="lastText">
            {lastText.trim() ? lastText : "-"}
          </span>
          <span className={`lastCopy ${lastHover && lastText.trim() ? "isOn" : ""}`}>
            COPY
          </span>
        </button>
      </div>
    </div>
  );
}
