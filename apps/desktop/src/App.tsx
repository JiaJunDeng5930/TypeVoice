import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type TaskEvent = {
  task_id: string;
  stage: string;
  status: "started" | "completed" | "failed" | "cancelled";
  message: string;
  elapsed_ms?: number | null;
  error_code?: string | null;
};

type TaskDone = {
  task_id: string;
  asr_text: string;
  final_text: string;
  rtf: number;
  device_used: string;
  preprocess_ms: number;
  asr_ms: number;
  rewrite_ms?: number | null;
  rewrite_enabled: boolean;
  template_id?: string | null;
};

type PromptTemplate = {
  id: string;
  name: string;
  system_prompt: string;
};

type Settings = {
  asr_model?: string | null;
  llm_base_url?: string | null;
  llm_model?: string | null;
  llm_reasoning_effort?: string | null;
};

type ModelStatus = {
  model_dir: string;
  ok: boolean;
  reason?: string | null;
};

type HistoryItem = {
  task_id: string;
  created_at_ms: number;
  asr_text: string;
  final_text: string;
  template_id?: string | null;
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
  const [taskId, setTaskId] = useState<string>("");
  const taskIdRef = useRef<string>("");
  const [events, setEvents] = useState<TaskEvent[]>([]);
  const [result, setResult] = useState<TaskDone | null>(null);
  const [editedText, setEditedText] = useState<string>("");

  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  const [templateId, setTemplateId] = useState<string>("");
  const [templateDraft, setTemplateDraft] = useState<string>("");
  const [rewriteEnabled, setRewriteEnabled] = useState<boolean>(false);
  const [llmKeyDraft, setLlmKeyDraft] = useState<string>("");
  const [llmStatus, setLlmStatus] = useState<string>("");
  const [llmBaseUrlDraft, setLlmBaseUrlDraft] = useState<string>("");
  const [llmModelDraft, setLlmModelDraft] = useState<string>("");
  const [llmReasoningEffortDraft, setLlmReasoningEffortDraft] =
    useState<string>("default");
  const [llmSettingsStatus, setLlmSettingsStatus] = useState<string>("");
  const [history, setHistory] = useState<HistoryItem[]>([]);

  const [templatesJson, setTemplatesJson] = useState<string>("");
  const [templatesIoStatus, setTemplatesIoStatus] = useState<string>("");

  const [asrModelDraft, setAsrModelDraft] = useState<string>("");
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [modelUiStatus, setModelUiStatus] = useState<string>("");

  const canCopy = useMemo(() => !!editedText.trim(), [editedText]);

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
    refreshHistory();
  }, []);

  useEffect(() => {
    const tpl = templates.find((x) => x.id === templateId);
    if (tpl) setTemplateDraft(tpl.system_prompt);
  }, [templateId, templates]);

  useEffect(() => {
    taskIdRef.current = taskId;
  }, [taskId]);

  useEffect(() => {
    let unlistenEvent: (() => void) | null = null;
    let unlistenDone: (() => void) | null = null;
    (async () => {
      unlistenEvent = await listen<TaskEvent>("task_event", (e) => {
        const ev = e.payload;
        if (!ev || ev.task_id !== taskIdRef.current) return;
        setEvents((prev) => {
          const next = [...prev, ev];
          return next.length > 200 ? next.slice(next.length - 200) : next;
        });
        if (ev.status === "failed") {
          // Non-fatal failures (e.g. Rewrite) still produce task_done.
          if (ev.stage !== "Rewrite") {
            setError(`${ev.stage}:${ev.error_code || "E_FAILED"}:${ev.message}`);
            setStatus("error");
            setTaskId("");
          }
        }
        if (ev.status === "cancelled") {
          setStatus("idle");
          setTaskId("");
        }
      });

      unlistenDone = await listen<TaskDone>("task_done", async (e) => {
        const done = e.payload;
        if (!done || done.task_id !== taskIdRef.current) return;
        setResult(done);
        setEditedText(done.final_text || done.asr_text || "");
        setStatus("done");
        setTaskId("");
        await refreshHistory();
      });
    })();

    return () => {
      try {
        unlistenEvent?.();
      } catch {
        // ignore
      }
      try {
        unlistenDone?.();
      } catch {
        // ignore
      }
    };
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const s = (await invoke("get_settings")) as Settings;
        setAsrModelDraft(String(s.asr_model || ""));
        setLlmBaseUrlDraft(String(s.llm_base_url || ""));
        setLlmModelDraft(String(s.llm_model || ""));
        setLlmReasoningEffortDraft(String(s.llm_reasoning_effort || "default"));
      } catch {
        // ignore
      }
      await refreshModelStatus();
    })();
  }, []);

  async function startFixture() {
    setStatus("running");
    setError("");
    setResult(null);
    setEditedText("");
    setEvents([]);
    try {
      const id = (await invoke("start_transcribe_fixture", {
        fixtureName: fixture,
        rewriteEnabled,
        templateId: templateId || null,
      })) as string;
      setTaskId(id);
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
    setEditedText("");
    setEvents([]);
    try {
      const b64 = await blobToBase64(recBlob);
      const id = (await invoke("start_transcribe_recording_base64", {
        b64,
        ext: "webm",
        rewriteEnabled,
        templateId: templateId || null,
      })) as string;
      setTaskId(id);
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  async function cancel() {
    const id = taskIdRef.current;
    if (!id) return;
    try {
      await invoke("cancel_task", { taskId: id });
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  async function copy() {
    if (!editedText.trim()) return;
    await navigator.clipboard.writeText(editedText);
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

  async function clearApiKey() {
    setLlmStatus("");
    try {
      await invoke("clear_llm_api_key");
      setLlmStatus("api_key_cleared");
    } catch (e) {
      setLlmStatus(`api_key_clear_failed: ${String(e)}`);
    }
  }

  async function checkApiKeyStatus() {
    setLlmStatus("");
    try {
      const st = (await invoke("llm_api_key_status")) as {
        configured: boolean;
        source: string;
        reason?: string | null;
      };
      setLlmStatus(
        st.configured
          ? `api_key_configured source=${st.source}`
          : `api_key_missing source=${st.source} reason=${st.reason || ""}`,
      );
    } catch (e) {
      setLlmStatus(`api_key_status_failed: ${String(e)}`);
    }
  }

  async function refreshHistory() {
    try {
      const h = (await invoke("history_list", { limit: 20 })) as HistoryItem[];
      setHistory(h);
    } catch (e) {
      // ignore
    }
  }

  async function clearHistory() {
    try {
      await invoke("history_clear");
      setHistory([]);
    } catch (e) {
      // ignore
    }
  }

  async function exportTemplates() {
    setTemplatesIoStatus("");
    try {
      const s = (await invoke("templates_export_json")) as string;
      setTemplatesJson(s);
      setTemplatesIoStatus("export_ok");
    } catch (e) {
      setTemplatesIoStatus(`export_failed:${String(e)}`);
    }
  }

  async function importTemplates(mode: "merge" | "replace") {
    setTemplatesIoStatus("");
    try {
      await invoke("templates_import_json", { json: templatesJson, mode });
      const t = (await invoke("list_templates")) as PromptTemplate[];
      setTemplates(t);
      setTemplatesIoStatus(`import_${mode}_ok`);
    } catch (e) {
      setTemplatesIoStatus(`import_failed:${String(e)}`);
    }
  }

  async function refreshModelStatus() {
    try {
      const st = (await invoke("asr_model_status")) as ModelStatus;
      setModelStatus(st);
      return st;
    } catch {
      return null;
    }
  }

  async function downloadModel() {
    setModelUiStatus("");
    try {
      const st = (await invoke("download_asr_model")) as ModelStatus;
      setModelStatus(st);
      setModelUiStatus(st.ok ? "download_ok" : `download_not_ok:${st.reason || ""}`);
      const s = (await invoke("get_settings")) as Settings;
      setAsrModelDraft(String(s.asr_model || ""));
    } catch (e) {
      setModelUiStatus(`download_failed:${String(e)}`);
    }
  }

  async function saveSettings() {
    setModelUiStatus("");
    try {
      await invoke("update_settings", {
        patch: { asr_model: asrModelDraft.trim() ? asrModelDraft.trim() : null },
      });
      setModelUiStatus("settings_saved");
    } catch (e) {
      setModelUiStatus(`settings_save_failed:${String(e)}`);
    }
  }

  async function saveLlmSettings() {
    setLlmSettingsStatus("");
    try {
      await invoke("update_settings", {
        patch: {
          llm_base_url: llmBaseUrlDraft.trim() ? llmBaseUrlDraft.trim() : null,
          llm_model: llmModelDraft.trim() ? llmModelDraft.trim() : null,
          llm_reasoning_effort:
            llmReasoningEffortDraft !== "default"
              ? llmReasoningEffortDraft
              : null,
        },
      });
      setLlmSettingsStatus("llm_settings_saved");
    } catch (e) {
      setLlmSettingsStatus(`llm_settings_save_failed:${String(e)}`);
    }
  }

  async function clearLlmSettings() {
    setLlmSettingsStatus("");
    try {
      await invoke("update_settings", {
        patch: { llm_base_url: null, llm_model: null, llm_reasoning_effort: null },
      });
      setLlmBaseUrlDraft("");
      setLlmModelDraft("");
      setLlmReasoningEffortDraft("default");
      setLlmSettingsStatus("llm_settings_cleared");
    } catch (e) {
      setLlmSettingsStatus(`llm_settings_clear_failed:${String(e)}`);
    }
  }

  return (
    <main className="container">
      <h1>TypeVoice (Dev)</h1>
      <p>
        当前页面用于 MVP 开发联调: 录音/fixtures 转录, 可选 LLM 改写, 以及一键复制.
      </p>

      <section className="card">
        <h2>ASR 模型</h2>
        <div className="row">
          <button onClick={downloadModel} disabled={status === "running"}>
            Download Model
          </button>
          <button onClick={refreshModelStatus} disabled={status === "running"}>
            Refresh
          </button>
          <div className="hint">
            {modelStatus
              ? `ok=${modelStatus.ok} dir=${modelStatus.model_dir} reason=${modelStatus.reason || ""}`
              : "status: unknown"}
          </div>
        </div>
        <div className="row">
          <label>
            settings.asr_model
            <input
              value={asrModelDraft}
              onChange={(e) => setAsrModelDraft(e.currentTarget.value)}
              placeholder="local dir or HF repo id"
            />
          </label>
          <button onClick={saveSettings} disabled={status === "running"}>
            Save Settings
          </button>
          <div className="hint">{modelUiStatus}</div>
        </div>
      </section>

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
            API Base URL
            <input
              value={llmBaseUrlDraft}
              onChange={(e) => setLlmBaseUrlDraft(e.currentTarget.value)}
              placeholder="https://api.openai.com/v1 (or paste /chat/completions)"
            />
          </label>
          <label>
            Model
            <input
              value={llmModelDraft}
              onChange={(e) => setLlmModelDraft(e.currentTarget.value)}
              placeholder="gpt-4o-mini"
            />
          </label>
          <label>
            推理等级
            <select
              value={llmReasoningEffortDraft}
              onChange={(e) => setLlmReasoningEffortDraft(e.currentTarget.value)}
            >
              <option value="default">default (不发送)</option>
              <option value="none">none</option>
              <option value="minimal">minimal</option>
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
              <option value="xhigh">xhigh</option>
            </select>
          </label>
          <button onClick={saveLlmSettings} disabled={status === "running"}>
            Save LLM Settings
          </button>
          <button onClick={clearLlmSettings} disabled={status === "running"}>
            Clear LLM Settings
          </button>
          <div className="hint">{llmSettingsStatus}</div>
        </div>

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
          <button onClick={clearApiKey}>
            Clear Key
          </button>
          <button onClick={checkApiKeyStatus}>
            Check Key
          </button>
          <div className="hint">{llmStatus}</div>
        </div>

        <div className="row" style={{ marginTop: 10 }}>
          <button onClick={exportTemplates} disabled={status === "running"}>
            Export Templates (JSON)
          </button>
          <button onClick={() => importTemplates("merge")} disabled={!templatesJson.trim() || status === "running"}>
            Import Merge
          </button>
          <button onClick={() => importTemplates("replace")} disabled={!templatesJson.trim() || status === "running"}>
            Import Replace
          </button>
          <div className="hint">{templatesIoStatus}</div>
        </div>

        <textarea
          value={templatesJson}
          onChange={(e) => setTemplatesJson(e.currentTarget.value)}
          placeholder="[ { id, name, system_prompt }, ... ]"
          style={{ height: 120, marginTop: 10 }}
        />
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

        <button onClick={startFixture} disabled={status === "running"}>
          {status === "running" ? "Running..." : "Transcribe"}
        </button>

        <button onClick={cancel} disabled={status !== "running" || !taskId}>
          Cancel
        </button>

        <button onClick={copy} disabled={!canCopy}>
          Copy
        </button>

        <div className="hint">
          task: {taskId || "-"}{" "}
          {events.length ? `stage=${events[events.length - 1]!.stage} status=${events[events.length - 1]!.status}` : ""}
        </div>
      </div>
      </section>

      {events.length ? (
        <section className="card">
          <h2>进度</h2>
          <div className="history">
            {events.slice().reverse().map((ev, idx) => (
              <div key={`${ev.stage}-${ev.status}-${idx}`} className="historyItem" style={{ cursor: "default" }}>
                <div className="historyTitle">
                  {ev.stage} {ev.status} {ev.elapsed_ms != null ? `(${ev.elapsed_ms}ms)` : ""}{" "}
                  {ev.error_code ? `[${ev.error_code}]` : ""}
                </div>
                <div className="historyPreview">{ev.message}</div>
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <section className="card">
        <h2>历史</h2>
        <div className="row">
          <button onClick={clearHistory} disabled={history.length === 0}>
            Clear
          </button>
          <div className="hint">{history.length} items</div>
        </div>
        <div className="history">
          {history.map((h) => (
            <button
              key={h.task_id}
              className="historyItem"
              onClick={() => {
                setResult({
                  task_id: h.task_id,
                  asr_text: h.asr_text,
                  final_text: h.final_text,
                  rtf: h.rtf,
                  device_used: h.device_used,
                  preprocess_ms: h.preprocess_ms,
                  asr_ms: h.asr_ms,
                  rewrite_ms: null,
                  rewrite_enabled: !!h.template_id,
                  template_id: h.template_id || null,
                });
                setEditedText(h.final_text || h.asr_text || "");
                setStatus("done");
              }}
            >
              <div className="historyTitle">
                {new Date(h.created_at_ms).toLocaleString()} ({h.device_used},{" "}
                rtf {h.rtf.toFixed(2)})
              </div>
              <div className="historyPreview">
                {(h.final_text || h.asr_text).slice(0, 80)}
              </div>
            </button>
          ))}
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
            value={editedText}
            onChange={(e) => setEditedText(e.currentTarget.value)}
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
