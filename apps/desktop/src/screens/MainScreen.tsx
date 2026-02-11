import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { blobToBase64, guessAudioExtFromMime } from "../lib/audio";
import { copyText } from "../lib/clipboard";
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
};

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
};

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "toString" in err) {
    try {
      return String(err);
    } catch {
      return "";
    }
  }
  return "";
}

function transcribeErrorHint(err: unknown): string {
  const raw = errorMessage(err);
  if (raw.includes("E_TOOLCHAIN_NOT_READY")) return "TOOLCHAIN NOT READY";
  if (raw.includes("E_TOOLCHAIN_CHECKSUM_MISMATCH")) return "TOOLCHAIN CHECKSUM ERROR";
  if (raw.includes("E_TOOLCHAIN_VERSION_MISMATCH")) return "TOOLCHAIN VERSION ERROR";
  if (raw.includes("E_PYTHON_NOT_READY")) return "PYTHON NOT READY";
  return "TRANSCRIBE FAILED";
}

export function MainScreen({ settings, pushToast, onHistoryChanged }: Props) {
  const [ui, setUi] = useState<UiState>("idle");
  const [hover, setHover] = useState(false);

  const [lastText, setLastText] = useState<string>("");
  const [lastMeta, setLastMeta] = useState<string>("NO LAST RESULT");
  const [lastHover, setLastHover] = useState(false);

  const uiRef = useRef<UiState>("idle");
  useEffect(() => {
    uiRef.current = ui;
  }, [ui]);

  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<BlobPart[]>([]);
  const mimeRef = useRef<string>("audio/webm");

  const activeTaskIdRef = useRef<string>("");
  const hotkeySessionRef = useRef<boolean>(false);

  const rewriteEnabled = settings?.rewrite_enabled === true;
  const templateId = settings?.rewrite_template_id || null;
  const hotkeysEnabled = settings?.hotkeys_enabled !== false;
  const showOverlay = settings?.hotkeys_show_overlay !== false;

  const hint = useMemo(() => {
    if (ui === "idle") return "START";
    if (ui === "recording") return "STOP";
    if (ui === "cancelling") return "CANCELLING";
    return "CANCEL";
  }, [ui]);

  async function overlaySet(visible: boolean, status: string, detail?: string | null) {
    if (!showOverlay) return;
    try {
      await invoke("overlay_set_state", {
        state: { visible, status, detail: detail || null, ts_ms: Date.now() },
      });
    } catch {
      // ignore
    }
  }

  function overlayFlash(status: string, ms: number, detail?: string | null) {
    void overlaySet(true, status, detail || null);
    window.setTimeout(() => {
      void overlaySet(false, "IDLE");
    }, ms);
  }

  useEffect(() => {
    (async () => {
      try {
        const runtime = (await invoke("runtime_toolchain_status")) as RuntimeToolchainStatus;
        if (!runtime.ready) {
          pushToast("TOOLCHAIN NOT READY", "danger");
        }
      } catch {
        // ignore
      }
      try {
        const runtime = (await invoke("runtime_python_status")) as RuntimePythonStatus;
        if (!runtime.ready) {
          pushToast("PYTHON NOT READY", "danger");
        }
      } catch {
        // ignore
      }

      try {
        const rows = (await invoke("history_list", {
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
    let unlistenDone: null | (() => void) = null;
    let unlistenEvent: null | (() => void) = null;
    let unlistenHotkey: null | (() => void) = null;
    (async () => {
      unlistenDone = await listen<TaskDone>("task_done", async (e) => {
        const done = e.payload;
        if (!done) return;
        if (done.task_id !== activeTaskIdRef.current) return;
        activeTaskIdRef.current = "";
        setUi("idle");

        const text = done.final_text || done.asr_text || "";
        setLastText(text);
        setLastMeta(new Date().toLocaleString());
        try {
          await copyText(text);
          pushToast("COPIED", "ok");
        } catch {
          pushToast("COPY FAILED", "danger");
        }
        if (hotkeySessionRef.current) {
          overlayFlash("COPIED", 800);
          hotkeySessionRef.current = false;
        }
        onHistoryChanged();
      });

      unlistenEvent = await listen<TaskEvent>("task_event", (e) => {
        const ev = e.payload;
        if (!ev) return;
        if (ev.task_id !== activeTaskIdRef.current) return;

        if (ev.status === "failed" && ev.stage !== "Rewrite") {
          activeTaskIdRef.current = "";
          setUi("idle");
          pushToast("ERROR", "danger");
          if (hotkeySessionRef.current) {
            overlayFlash("ERROR", 1200);
            hotkeySessionRef.current = false;
          }
        }
        if (ev.status === "failed" && ev.stage === "Rewrite") {
          pushToast("REWRITE FAILED", "danger");
        }
        if (ev.status === "cancelled") {
          activeTaskIdRef.current = "";
          setUi("idle");
          pushToast("CANCELLED", "default");
          if (hotkeySessionRef.current) {
            overlayFlash("CANCELLED", 800);
            hotkeySessionRef.current = false;
          }
        }
      });

      unlistenHotkey = await listen<HotkeyRecordEvent>("tv_hotkey_record", (e) => {
        if (!hotkeysEnabled) return;
        const hk = e.payload;
        if (!hk) return;

        const cur = uiRef.current;
        if (hk.kind === "ptt") {
          if (hk.state === "Pressed" && cur === "idle") {
            void startRecording("hotkey");
          }
          if (hk.state === "Released" && cur === "recording") {
            void stopAndTranscribe();
          }
          return;
        }

        // toggle
        if (hk.state !== "Pressed") return;
        if (cur === "idle") void startRecording("hotkey");
        else if (cur === "recording") void stopAndTranscribe();
        else if (cur === "transcribing") void cancelActiveTask();
      });
    })();
    return () => {
      try {
        unlistenDone?.();
      } catch {
        // ignore
      }
      try {
        unlistenEvent?.();
      } catch {
        // ignore
      }
      try {
        unlistenHotkey?.();
      } catch {
        // ignore
      }
    };
  }, [hotkeysEnabled, onHistoryChanged, pushToast, rewriteEnabled, showOverlay, templateId]);

  async function startRecording(source: "ui" | "hotkey" = "ui") {
    chunksRef.current = [];
    hotkeySessionRef.current = source === "hotkey";
    if (hotkeySessionRef.current) void overlaySet(true, "REC");
    // Snapshot settings at the moment recording starts.
    const rewriteEnabledNow = rewriteEnabled;
    const templateIdNow = templateId;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;

      const mimeType =
        MediaRecorder.isTypeSupported("audio/webm;codecs=opus") &&
        "audio/webm;codecs=opus";
      const r = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
      mimeRef.current = r.mimeType || "audio/webm";
      recorderRef.current = r;

      r.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) chunksRef.current.push(e.data);
      };

      r.onstop = async () => {
        stream.getTracks().forEach((t) => t.stop());
        streamRef.current = null;
        recorderRef.current = null;

        const blob = new Blob(chunksRef.current, { type: mimeRef.current });
        chunksRef.current = [];

        try {
          if (hotkeySessionRef.current) void overlaySet(true, "TRANSCRIBING");
          const b64 = await blobToBase64(blob);
          const ext = guessAudioExtFromMime(mimeRef.current);
          const id = (await invoke("start_transcribe_recording_base64", {
            b64,
            ext,
            rewriteEnabled: rewriteEnabledNow,
            templateId: templateIdNow,
          })) as string;
          activeTaskIdRef.current = id;
        } catch (err) {
          activeTaskIdRef.current = "";
          setUi("idle");
          const hint = transcribeErrorHint(err);
          pushToast(hint, "danger");
          if (hotkeySessionRef.current) {
            overlayFlash("ERROR", 1200, hint);
            hotkeySessionRef.current = false;
          }
        }
      };

      r.start();
      setUi("recording");
    } catch {
      setUi("idle");
      pushToast("MIC PERMISSION NEEDED", "danger");
      if (hotkeySessionRef.current) {
        overlayFlash("MIC DENIED", 1400);
        hotkeySessionRef.current = false;
      }
    }
  }

  async function stopAndTranscribe() {
    const r = recorderRef.current;
    if (!r) return;
    setUi("transcribing");
    try {
      r.stop();
    } catch {
      setUi("idle");
      pushToast("STOP FAILED", "danger");
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, "STOP FAILED");
        hotkeySessionRef.current = false;
      }
    }
  }

  async function cancelActiveTask() {
    const id = activeTaskIdRef.current;
    if (!id) return;
    setUi("cancelling");
    try {
      await invoke("cancel_task", { taskId: id });
      pushToast("CANCELLING...", "default");
      if (hotkeySessionRef.current) void overlaySet(true, "CANCELLING");
    } catch {
      setUi("transcribing");
      pushToast("CANCEL FAILED", "danger");
      if (hotkeySessionRef.current) {
        overlayFlash("ERROR", 1200, "CANCEL FAILED");
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
      await copyText(lastText);
      pushToast("COPIED", "ok");
    } catch {
      pushToast("COPY FAILED", "danger");
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
