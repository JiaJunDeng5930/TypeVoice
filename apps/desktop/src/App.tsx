import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type TranscribeResult = {
  task_id: string;
  asr_text: string;
  rtf: number;
  device_used: string;
  preprocess_ms: number;
  asr_ms: number;
};

const FIXTURES = [
  { id: "zh_10s.ogg", label: "中文 10s" },
  { id: "zh_60s.ogg", label: "中文 60s" },
  { id: "zh_5m.ogg", label: "中文 5min" },
];

function App() {
  const [recState, setRecState] = useState<
    "idle" | "recording" | "stopping" | "ready"
  >("idle");
  const [recError, setRecError] = useState<string>("");
  const [recBlob, setRecBlob] = useState<Blob | null>(null);
  const [recorder, setRecorder] = useState<MediaRecorder | null>(null);

  const [fixture, setFixture] = useState(FIXTURES[0]!.id);
  const [status, setStatus] = useState<"idle" | "running" | "done" | "error">(
    "idle",
  );
  const [error, setError] = useState<string>("");
  const [result, setResult] = useState<TranscribeResult | null>(null);

  const canCopy = useMemo(() => !!result?.asr_text?.trim(), [result]);

  async function run() {
    setStatus("running");
    setError("");
    setResult(null);
    try {
      const r = (await invoke("transcribe_fixture", {
        fixtureName: fixture,
      })) as TranscribeResult;
      setResult(r);
      setStatus("done");
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  async function startRecording() {
    setRecError("");
    setRecBlob(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType =
        MediaRecorder.isTypeSupported("audio/webm;codecs=opus") &&
        "audio/webm;codecs=opus";
      const r = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
      const chunks: BlobPart[] = [];
      r.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) chunks.push(e.data);
      };
      r.onstop = () => {
        stream.getTracks().forEach((t) => t.stop());
        const blob = new Blob(chunks, { type: r.mimeType || "audio/webm" });
        setRecBlob(blob);
        setRecState("ready");
        setRecorder(null);
      };
      r.start();
      setRecorder(r);
      setRecState("recording");
    } catch (e) {
      setRecError(String(e));
      setRecState("idle");
    }
  }

  async function stopRecording() {
    if (!recorder) return;
    setRecState("stopping");
    recorder.stop();
  }

  async function transcribeRecording() {
    if (!recBlob) return;
    setStatus("running");
    setError("");
    setResult(null);
    try {
      const b64 = await blobToBase64(recBlob);
      const r = (await invoke("transcribe_recording_base64", {
        b64,
        ext: "webm",
      })) as TranscribeResult;
      setResult(r);
      setStatus("done");
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  async function copy() {
    if (!result?.asr_text) return;
    await navigator.clipboard.writeText(result.asr_text);
  }

  return (
    <main className="container">
      <h1>TypeVoice (Dev)</h1>
      <p>
        当前页面用于 M3 验收: 运行 fixtures 转录并展示结果, 以及一键复制.
      </p>

      <section className="card">
        <h2>录音</h2>
        <div className="row">
          <button onClick={startRecording} disabled={recState === "recording"}>
            Start
          </button>
          <button onClick={stopRecording} disabled={recState !== "recording"}>
            Stop
          </button>
          <button
            onClick={transcribeRecording}
            disabled={!recBlob || status === "running"}
          >
            Transcribe Recording
          </button>
          <div className="hint">
            state: {recState} {recBlob ? `(blob ${Math.round(recBlob.size / 1024)}KB)` : ""}
          </div>
        </div>
        {recError ? <pre className="error">{recError}</pre> : null}
      </section>

      <section className="card">
        <h2>Fixtures</h2>
      <div className="row">
        <label>
          Fixture
          <select
            value={fixture}
            onChange={(e) => setFixture(e.currentTarget.value)}
          >
            {FIXTURES.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
        </label>

        <button onClick={run} disabled={status === "running"}>
          {status === "running" ? "Running..." : "Transcribe"}
        </button>

        <button onClick={copy} disabled={!canCopy}>
          Copy
        </button>
      </div>
      </section>

      {status === "error" ? (
        <pre className="error">{error}</pre>
      ) : result ? (
        <section className="result">
          <div className="meta">
            <div>task_id: {result.task_id}</div>
            <div>
              device: {result.device_used} rtf: {result.rtf.toFixed(3)}
            </div>
            <div>
              preprocess_ms: {result.preprocess_ms} asr_ms: {result.asr_ms}
            </div>
          </div>
          <textarea readOnly value={result.asr_text} />
        </section>
      ) : null}
    </main>
  );
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("FileReader error"));
    reader.onload = () => {
      const s = String(reader.result || "");
      const comma = s.indexOf(",");
      resolve(comma >= 0 ? s.slice(comma + 1) : s);
    };
    reader.readAsDataURL(blob);
  });
}

export default App;
