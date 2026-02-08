import { useEffect, useMemo, useState } from "react";
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

type PromptTemplate = {
  id: string;
  name: string;
  system_prompt: string;
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
  const [finalText, setFinalText] = useState<string>("");

  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  const [templateId, setTemplateId] = useState<string>("");
  const [templateDraft, setTemplateDraft] = useState<string>("");
  const [rewriteEnabled, setRewriteEnabled] = useState<boolean>(false);
  const [llmKeyDraft, setLlmKeyDraft] = useState<string>("");
  const [llmStatus, setLlmStatus] = useState<string>("");

  const canCopy = useMemo(() => !!(finalText || result?.asr_text)?.trim(), [finalText, result]);

  useEffect(() => {
    (async () => {
      try {
        const t = (await invoke("list_templates")) as PromptTemplate[];
        setTemplates(t);
        const first = t[0]?.id || "";
        setTemplateId(first);
        setTemplateDraft(t[0]?.system_prompt || "");
      } catch (e) {
        // templates are optional for pure ASR usage
        setLlmStatus(`templates_load_failed: ${String(e)}`);
      }
    })();
  }, []);

  useEffect(() => {
    const tpl = templates.find((x) => x.id === templateId);
    if (tpl) setTemplateDraft(tpl.system_prompt);
  }, [templateId, templates]);

  async function run() {
    setStatus("running");
    setError("");
    setResult(null);
    setFinalText("");
    try {
      const r = (await invoke("transcribe_fixture", {
        fixtureName: fixture,
      })) as TranscribeResult;
      setResult(r);
      if (rewriteEnabled && templateId) {
        try {
          const rewritten = (await invoke("rewrite_text", {
            templateId,
            asrText: r.asr_text,
          })) as string;
          setFinalText(rewritten);
        } catch (e) {
          setLlmStatus(`rewrite_failed: ${String(e)}`);
          setFinalText("");
        }
      }
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
    setFinalText("");
    try {
      const b64 = await blobToBase64(recBlob);
      const r = (await invoke("transcribe_recording_base64", {
        b64,
        ext: "webm",
      })) as TranscribeResult;
      setResult(r);
      if (rewriteEnabled && templateId) {
        try {
          const rewritten = (await invoke("rewrite_text", {
            templateId,
            asrText: r.asr_text,
          })) as string;
          setFinalText(rewritten);
        } catch (e) {
          setLlmStatus(`rewrite_failed: ${String(e)}`);
          setFinalText("");
        }
      }
      setStatus("done");
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  async function copy() {
    const text = finalText || result?.asr_text || "";
    if (!text.trim()) return;
    await navigator.clipboard.writeText(text);
  }

  async function saveTemplate() {
    if (!templateId) return;
    const tpl = templates.find((x) => x.id === templateId);
    if (!tpl) return;
    setLlmStatus("");
    try {
      const updated = (await invoke("upsert_template", {
        tpl: { ...tpl, system_prompt: templateDraft },
      })) as PromptTemplate;
      setTemplates((prev) =>
        prev.map((x) => (x.id === updated.id ? updated : x)),
      );
      setLlmStatus("template_saved");
    } catch (e) {
      setLlmStatus(`template_save_failed: ${String(e)}`);
    }
  }

  async function setApiKey() {
    setLlmStatus("");
    try {
      await invoke("set_llm_api_key", { apiKey: llmKeyDraft });
      setLlmKeyDraft("");
      setLlmStatus("api_key_saved");
    } catch (e) {
      setLlmStatus(`api_key_save_failed: ${String(e)}`);
    }
  }

  return (
    <main className="container">
      <h1>TypeVoice (Dev)</h1>
      <p>
        当前页面用于 MVP 开发联调: 录音/fixtures 转录, 可选 LLM 改写, 以及一键复制.
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
        <h2>LLM 改写</h2>
        <div className="row">
          <label>
            Template
            <select
              value={templateId}
              onChange={(e) => setTemplateId(e.currentTarget.value)}
              disabled={templates.length === 0}
            >
              {templates.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </label>

          <label>
            Enable
            <input
              type="checkbox"
              checked={rewriteEnabled}
              onChange={(e) => setRewriteEnabled(e.currentTarget.checked)}
            />
          </label>

          <button onClick={saveTemplate} disabled={!templateId}>
            Save Template
          </button>
        </div>

        <textarea
          value={templateDraft}
          onChange={(e) => setTemplateDraft(e.currentTarget.value)}
          placeholder="system prompt..."
          style={{ height: 160 }}
        />

        <div className="row" style={{ marginTop: 10 }}>
          <label>
            API Key
            <input
              value={llmKeyDraft}
              onChange={(e) => setLlmKeyDraft(e.currentTarget.value)}
              placeholder="save to keyring (or set TYPEVOICE_LLM_API_KEY)"
            />
          </label>
          <button onClick={setApiKey} disabled={!llmKeyDraft.trim()}>
            Save Key
          </button>
          <div className="hint">{llmStatus}</div>
        </div>
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
          <textarea
            readOnly
            value={finalText || result.asr_text}
            placeholder="result..."
          />
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
