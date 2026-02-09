import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { blobToBase64, guessAudioExtFromMime } from "../lib/audio";
import { copyText } from "../lib/clipboard";
import type { HistoryItem, Settings, TaskDone, TaskEvent } from "../types";
import { IconStart, IconStop, IconTranscribing } from "../ui/icons";

type UiState = "idle" | "recording" | "transcribing" | "cancelling";

type Props = {
  settings: Settings | null;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryChanged: () => void;
};

export function MainScreen({ settings, pushToast, onHistoryChanged }: Props) {
  const [ui, setUi] = useState<UiState>("idle");
  const [hover, setHover] = useState(false);

  const [lastText, setLastText] = useState<string>("");
  const [lastMeta, setLastMeta] = useState<string>("NO LAST RESULT");
  const [lastHover, setLastHover] = useState(false);

  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<BlobPart[]>([]);
  const mimeRef = useRef<string>("audio/webm");

  const activeTaskIdRef = useRef<string>("");

  const rewriteEnabled = settings?.rewrite_enabled === true;
  const templateId = settings?.rewrite_template_id || null;

  const hint = useMemo(() => {
    if (ui === "idle") return "START";
    if (ui === "recording") return "STOP";
    if (ui === "cancelling") return "CANCELLING";
    return "CANCEL";
  }, [ui]);

  useEffect(() => {
    (async () => {
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
        }
        if (ev.status === "failed" && ev.stage === "Rewrite") {
          pushToast("REWRITE FAILED", "danger");
        }
        if (ev.status === "cancelled") {
          activeTaskIdRef.current = "";
          setUi("idle");
          pushToast("CANCELLED", "default");
        }
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
    };
  }, [onHistoryChanged, pushToast]);

  async function startRecording() {
    chunksRef.current = [];
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
          const b64 = await blobToBase64(blob);
          const ext = guessAudioExtFromMime(mimeRef.current);
          const id = (await invoke("start_transcribe_recording_base64", {
            b64,
            ext,
            rewriteEnabled,
            templateId,
          })) as string;
          activeTaskIdRef.current = id;
        } catch {
          activeTaskIdRef.current = "";
          setUi("idle");
          pushToast("TRANSCRIBE FAILED", "danger");
        }
      };

      r.start();
      setUi("recording");
    } catch {
      setUi("idle");
      pushToast("MIC PERMISSION NEEDED", "danger");
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
    }
  }

  async function cancelActiveTask() {
    const id = activeTaskIdRef.current;
    if (!id) return;
    setUi("cancelling");
    try {
      await invoke("cancel_task", { taskId: id });
      pushToast("CANCELLING...", "default");
    } catch {
      setUi("transcribing");
      pushToast("CANCEL FAILED", "danger");
    }
  }

  async function onMainButtonClick() {
    if (ui === "idle") return startRecording();
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
