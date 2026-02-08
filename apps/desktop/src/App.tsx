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

export default App;
