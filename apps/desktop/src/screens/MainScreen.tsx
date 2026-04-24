import { useEffect, useMemo, useRef, useState } from "react";
import {
  browserClipboard,
  browserTimer,
  defaultTauriGateway,
  type ClipboardPort,
  type TauriGateway,
  type TimerPort,
} from "../infra/runtimePorts";
import { createBackendClient, type BackendClient } from "../infra/backendClient";
import {
  buildDiagnostic,
  compactDetail,
  hotkeyCaptureHint,
  type DiagnosticView,
  toDiagnosticLine,
} from "../domain/diagnostic";
import type {
  HistoryItem,
  RuntimePythonStatus,
  RuntimeToolchainStatus,
  Settings,
  UiEvent,
} from "../types";
import { IconStart, IconStop, IconTranscribing } from "../ui/icons";

type UiState = "idle" | "recording" | "transcribing" | "rewriting" | "inserting" | "cancelling";

type HotkeyRecordEvent = {
  kind: "ptt" | "toggle";
  state: "Pressed" | "Released";
  shortcut: string;
  ts_ms: number;
  task_id?: string | null;
  capture_status?: "ok" | "err" | null;
  capture_error_code?: string | null;
  capture_error_message?: string | null;
};

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
  gateway?: TauriGateway;
  timer?: TimerPort;
  clipboard?: ClipboardPort;
  backend?: BackendClient;
};

export function MainScreen({
  settings,
  pushToast,
  onHistoryChanged,
  gateway = defaultTauriGateway,
  timer = browserTimer,
  clipboard = browserClipboard,
  backend,
}: Props) {
  const client = useMemo(() => backend || createBackendClient(gateway), [backend, gateway]);
  const [ui, setUi] = useState<UiState>("idle");
  const [hover, setHover] = useState(false);
  const [diagnosticLine, setDiagnosticLine] = useState<string>("");

  const [lastText, setLastText] = useState<string>("");
  const [lastAsrText, setLastAsrText] = useState<string>("");
  const [lastTranscriptId, setLastTranscriptId] = useState<string>("");
  const [lastMeta, setLastMeta] = useState("NO LAST RESULT");
  const [lastHover, setLastHover] = useState(false);

  const uiRef = useRef<UiState>("idle");
  const activeSessionIdRef = useRef("");
  const activeTranscriptIdRef = useRef("");
  const hotkeySessionRef = useRef(false);
  const pendingTaskIdRef = useRef<string | null>(null);

  const hasHotkeyConfig =
    typeof settings?.hotkeys_enabled === "boolean" &&
    typeof settings?.hotkeys_show_overlay === "boolean";
  const hotkeysEnabledRef = useRef(false);
  const showOverlayRef = useRef(false);
  const hasHotkeyConfigRef = useRef(hasHotkeyConfig);
  const pushToastRef = useRef(pushToast);
  const onHistoryChangedRef = useRef(onHistoryChanged);

  useEffect(() => {
    uiRef.current = ui;
  }, [ui]);

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
    if (ui === "transcribing") return "CANCEL";
    if (ui === "rewriting") return "REWRITING";
    if (ui === "inserting") return "INSERTING";
    return "CANCELLING";
  }, [ui]);

  async function overlaySet(visible: boolean, status: string, detail?: string | null) {
    if (!showOverlayRef.current) return;
    try {
      await client.overlaySetState({
        visible,
        status,
        detail: detail || null,
        ts_ms: Date.now(),
      });
    } catch {
    }
  }

  function overlayFlash(status: string, ms: number, detail?: string | null) {
    void overlaySet(true, status, detail || null);
    timer.setTimeout(() => {
      void overlaySet(false, "IDLE");
    }, ms);
  }

  function waitMs(ms: number): Promise<void> {
    return new Promise((resolve) => {
      timer.setTimeout(resolve, ms);
    });
  }

  async function abortPendingTaskBestEffort(taskId: string | null) {
    if (!taskId || !taskId.trim()) return;
    try {
      await client.abortPendingTask(taskId);
    } catch {
    }
  }

  async function logUiEventBestEffort(req: Record<string, unknown>) {
    try {
      await client.logUiEvent(req);
    } catch {
    }
  }

  function logDiagnosticBestEffort(diag: DiagnosticView, extra?: Record<string, unknown>) {
    void logUiEventBestEffort({
      kind: "diagnostic",
      code: diag.code,
      title: diag.title,
      detail: diag.detail,
      actionHint: diag.actionHint,
      screen: "main",
      tab: "main",
      tsMs: Date.now(),
      extra: extra || null,
    });
  }

  function failFromEvent(ev: UiEvent, fallbackTitle: string) {
    const code = ev.errorCode || "E_TASK_FAILED";
    const detail = compactDetail([code, ev.message || fallbackTitle].filter(Boolean).join(": "));
    const diag: DiagnosticView = {
      code,
      title: fallbackTitle,
      detail,
      actionHint: "CHECK TRACE.JSONL WITH THIS ERROR CODE",
    };
    logDiagnosticBestEffort(diag, {
      source: "ui_event",
      taskStage: ev.stage || null,
      taskStatus: ev.status || null,
      taskId: ev.taskId || null,
    });
    pushToastRef.current(diag.title, "danger");
    setDiagnosticLine(toDiagnosticLine(diag));
    if (hotkeySessionRef.current) {
      overlayFlash("ERROR", 1400, diag.code);
      hotkeySessionRef.current = false;
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
        const runtime = await client.runtimeToolchainStatus() as RuntimeToolchainStatus;
        if (!runtime.ready) pushToast("TOOLCHAIN NOT READY", "danger");
      } catch {
      }
      try {
        const runtime = await client.runtimePythonStatus() as RuntimePythonStatus;
        if (!runtime.ready) pushToast("PYTHON NOT READY", "danger");
      } catch {
      }

      try {
        const rows = await client.historyList({ limit: 1, beforeMs: null }) as HistoryItem[];
        const h = rows[0];
        if (!h) return;
        const text = h.final_text || h.asr_text || "";
        setLastText(text);
        setLastAsrText(h.asr_text || text);
        setLastTranscriptId(h.task_id);
        activeTranscriptIdRef.current = h.task_id;
        setLastMeta(new Date(h.created_at_ms).toLocaleString());
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
      const unlistenUiEvent = await client.listenUiEvent((ev) => {
        if (!ev || ev.kind === "audio.level") return;
        const activeId = activeTranscriptIdRef.current || pendingTaskIdRef.current || "";
        if (ev.taskId && activeId && ev.taskId !== activeId) return;

        if (ev.status === "failed") {
          activeTranscriptIdRef.current = "";
          setUi("idle");
          failFromEvent(ev, ev.stage === "Rewrite" ? "REWRITE FAILED" : "TRANSCRIBE FAILED");
        }
        if (ev.status === "cancelled") {
          activeTranscriptIdRef.current = "";
          setUi("idle");
          pushToastRef.current("CANCELLED", "default");
          setDiagnosticLine("");
          if (hotkeySessionRef.current) {
            overlayFlash("CANCELLED", 800);
            hotkeySessionRef.current = false;
          }
        }
      });
      trackUnlisten(unlistenUiEvent);

      const unlistenHotkey = await client.listenHotkeyRecord<HotkeyRecordEvent>((hk) => {
        if (!hasHotkeyConfigRef.current) {
          pushToastRef.current("SETTINGS INVALID", "danger");
          return;
        }
        if (!hotkeysEnabledRef.current) return;
        if (!hk) return;

        const cur = uiRef.current;
        if (hk.kind === "ptt") {
          if (hk.state === "Pressed" && cur === "idle") {
            if (hk.capture_status !== "ok" || !hk.task_id) {
              showHotkeyCaptureError(hk);
              return;
            }
            void startRecording("hotkey", hk.task_id);
          }
          if (hk.state === "Released" && cur === "recording") {
            void stopAndTranscribe();
          }
          return;
        }

        if (hk.state !== "Pressed") return;
        if (cur === "idle") {
          if (hk.capture_status !== "ok" || !hk.task_id) {
            showHotkeyCaptureError(hk);
            return;
          }
          void startRecording("hotkey", hk.task_id);
        } else if (cur === "recording") {
          void stopAndTranscribe();
        } else if (cur === "transcribing") {
          void cancelActiveTask();
        }
      });
      trackUnlisten(unlistenHotkey);
    })().catch(() => {
    });
    return () => {
      cancelled = true;
      const sessionId = activeSessionIdRef.current;
      const transcriptId = activeTranscriptIdRef.current || pendingTaskIdRef.current;
      activeSessionIdRef.current = "";
      activeTranscriptIdRef.current = "";
      if (sessionId || transcriptId) {
        void client.recordTranscribeCancel({ sessionId, transcriptId }).catch(() => {});
      }
      const staleTaskId = pendingTaskIdRef.current;
      pendingTaskIdRef.current = null;
      void abortPendingTaskBestEffort(staleTaskId);
      for (const fn of unlistenFns) {
        try {
          fn();
        } catch {
        }
      }
    };
  }, [client]);

  function showHotkeyCaptureError(hk: HotkeyRecordEvent) {
    const captureCode = hk.capture_error_code || "E_HOTKEY_EVENT_INCOMPLETE";
    const hintText = hotkeyCaptureHint(captureCode);
    const detail = compactDetail(
      [captureCode, hk.capture_error_message || hintText].filter(Boolean).join(": "),
    );
    void logUiEventBestEffort({
      kind: "diagnostic",
      code: captureCode,
      title: hintText,
      detail,
      actionHint: "CHECK TRACE.JSONL WITH THIS ERROR CODE",
      screen: "main",
      tab: "main",
      triggerSource: "hotkey",
      tsMs: Date.now(),
      extra: {
        hotkeyKind: hk.kind,
        hotkeyState: hk.state,
        captureStatus: hk.capture_status || null,
        hasTaskId: !!hk.task_id,
        shortcut: hk.shortcut,
      },
    });
    setDiagnosticLine(detail);
    pushToastRef.current(hintText, "danger");
    void overlaySet(true, "ERROR", hintText);
    timer.setTimeout(() => {
      void overlaySet(false, "IDLE");
    }, 1200);
  }

  async function startRecording(source: "ui" | "hotkey" = "ui", taskId: string | null = null) {
    hotkeySessionRef.current = source === "hotkey";
    pendingTaskIdRef.current = source === "hotkey" ? taskId : null;
    activeTranscriptIdRef.current = taskId || "";
    setDiagnosticLine("");
    try {
      const started = await client.recordTranscribeStart({ taskId });
      activeSessionIdRef.current = started.sessionId;
      setUi("recording");
      if (hotkeySessionRef.current) void overlaySet(true, "REC");
    } catch (err) {
      const staleTaskId = pendingTaskIdRef.current;
      void abortPendingTaskBestEffort(staleTaskId);
      setUi("idle");
      const diag = buildDiagnostic(err, "RECORDING FAILED");
      logDiagnosticBestEffort(diag, { source: "record_transcribe_start" });
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      pendingTaskIdRef.current = null;
      activeTranscriptIdRef.current = "";
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function stopAndTranscribe() {
    const sessionId = activeSessionIdRef.current;
    if (!sessionId) return;
    setUi("transcribing");
    try {
      if (hotkeySessionRef.current) void overlaySet(true, "TRANSCRIBING");
      const result = await client.recordTranscribeStop({ sessionId });
      activeSessionIdRef.current = "";
      activeTranscriptIdRef.current = result.transcriptId;
      pendingTaskIdRef.current = null;
      setLastTranscriptId(result.transcriptId);
      setLastAsrText(result.asrText);
      setLastText(result.finalText || result.asrText || "");
      setLastMeta(new Date().toLocaleString());
      setUi("idle");
      setDiagnosticLine("");
      pushToastRef.current("TRANSCRIBED", "ok");
      if (hotkeySessionRef.current) {
        await overlaySet(false, "IDLE");
        await waitMs(60);
        overlayFlash("READY", 800);
        hotkeySessionRef.current = false;
      }
      onHistoryChangedRef.current();
    } catch (err) {
      const staleTaskId = pendingTaskIdRef.current;
      void abortPendingTaskBestEffort(staleTaskId);
      activeSessionIdRef.current = "";
      activeTranscriptIdRef.current = "";
      pendingTaskIdRef.current = null;
      setUi("idle");
      const diag = buildDiagnostic(err, "TRANSCRIBE FAILED");
      logDiagnosticBestEffort(diag, { source: "record_transcribe_stop" });
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function cancelActiveTask() {
    const sessionId = activeSessionIdRef.current;
    const transcriptId = activeTranscriptIdRef.current || pendingTaskIdRef.current;
    if (!sessionId && !transcriptId) return;
    setUi("cancelling");
    try {
      await client.recordTranscribeCancel({ sessionId, transcriptId });
      pushToastRef.current("CANCELLING...", "default");
      setDiagnosticLine("");
      activeSessionIdRef.current = "";
      activeTranscriptIdRef.current = "";
      pendingTaskIdRef.current = null;
      setUi("idle");
      if (hotkeySessionRef.current) void overlaySet(true, "CANCELLING");
    } catch (err) {
      setUi("transcribing");
      const diag = buildDiagnostic(err, "CANCEL FAILED");
      logDiagnosticBestEffort(diag, { source: "record_transcribe_cancel", transcriptId });
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, diag.code);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function rewriteLast() {
    if (!lastTranscriptId || !lastAsrText.trim()) return;
    setUi("rewriting");
    setDiagnosticLine("");
    try {
      const result = await client.rewriteText({
        transcriptId: lastTranscriptId,
        text: lastAsrText,
        templateId: settings?.rewrite_template_id || null,
      });
      setLastText(result.finalText);
      setLastMeta(new Date().toLocaleString());
      setUi("idle");
      pushToastRef.current("REWRITTEN", "ok");
      onHistoryChangedRef.current();
    } catch (err) {
      setUi("idle");
      const diag = buildDiagnostic(err, "REWRITE FAILED");
      logDiagnosticBestEffort(diag, { source: "rewrite_text", transcriptId: lastTranscriptId });
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
    }
  }

  async function insertLast() {
    if (!lastText.trim()) return;
    setUi("inserting");
    setDiagnosticLine("");
    try {
      const inserted = await client.insertText({
        transcriptId: lastTranscriptId || null,
        text: lastText,
      });
      setUi("idle");
      if (inserted.autoPasteAttempted && !inserted.autoPasteOk) {
        const code = inserted.errorCode || "E_EXPORT_PASTE_FAILED";
        const detail = compactDetail(
          [code, inserted.errorMessage || "auto paste failed"].filter(Boolean).join(": "),
        );
        setDiagnosticLine(detail);
        pushToastRef.current(`AUTO PASTE FAILED: ${code}`, "danger");
        return;
      }
      pushToastRef.current(inserted.autoPasteAttempted ? "COPIED + PASTED" : "COPIED", "ok");
    } catch (err) {
      setUi("idle");
      const diag = buildDiagnostic(err, "INSERT FAILED");
      logDiagnosticBestEffort(diag, { source: "insert_text", transcriptId: lastTranscriptId });
      pushToastRef.current(diag.title, "danger");
      setDiagnosticLine(toDiagnosticLine(diag));
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

  const actionBusy = ui !== "idle";
  const hasRewriteTemplate = settings?.rewrite_enabled === true && !!settings?.rewrite_template_id?.trim();
  const canRewrite = !!lastTranscriptId && !!lastAsrText.trim() && hasRewriteTemplate && !actionBusy;
  const canInsert = !!lastText.trim() && !actionBusy;

  return (
    <div className="card mainCard">
      <div className="mainCenter">
        <button
          type="button"
          className={`mainButton ${ui === "transcribing" || ui === "cancelling" ? "isBusy" : ""}`}
          onClick={onMainButtonClick}
          disabled={ui === "cancelling" || ui === "rewriting" || ui === "inserting"}
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
          {hover || ui !== "idle" ? hint : ""}
        </div>
        <div
          className={`mainDiag ${diagnosticLine ? "isVisible" : ""}`}
          aria-hidden={!diagnosticLine}
        >
          {diagnosticLine || ""}
        </div>

        <div className="mainActions">
          <button type="button" className="mainActionButton" onClick={rewriteLast} disabled={!canRewrite}>
            REWRITE
          </button>
          <button type="button" className="mainActionButton" onClick={insertLast} disabled={!canInsert}>
            INSERT
          </button>
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
