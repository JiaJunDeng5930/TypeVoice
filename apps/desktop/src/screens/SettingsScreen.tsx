import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import type { ApiKeyStatus, ModelStatus, PromptTemplate, Settings } from "../types";
import { PixelButton } from "../ui/PixelButton";
import { PixelDialog } from "../ui/PixelDialog";
import { PixelInput, PixelTextarea } from "../ui/PixelInput";
import { PixelSelect, type PixelSelectOption } from "../ui/PixelSelect";
import { PixelToggle } from "../ui/PixelToggle";

type Props = {
  settings: Settings | null;
  savePatch: (patch: Record<string, unknown>) => Promise<void>;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryCleared: () => void;
};

const REASONING: PixelSelectOption[] = [
  { value: "default", label: "default (omit)" },
  { value: "none", label: "none" },
  { value: "minimal", label: "minimal" },
  { value: "low", label: "low" },
  { value: "medium", label: "medium" },
  { value: "high", label: "high" },
  { value: "xhigh", label: "xhigh" },
];

export function SettingsScreen({
  settings,
  savePatch,
  pushToast,
  onHistoryCleared,
}: Props) {
  const [asrModel, setAsrModel] = useState("");
  const [llmBaseUrl, setLlmBaseUrl] = useState("");
  const [llmModel, setLlmModel] = useState("");
  const [reasoning, setReasoning] = useState("default");
  const [rewriteEnabled, setRewriteEnabled] = useState(false);
  const [rewriteTemplateId, setRewriteTemplateId] = useState("");

  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);

  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  const [tplId, setTplId] = useState("");
  const [tplDraft, setTplDraft] = useState("");

  const [keyDraft, setKeyDraft] = useState("");
  const [templatesJson, setTemplatesJson] = useState("");

  const [confirmClear, setConfirmClear] = useState(false);

  useEffect(() => {
    setAsrModel(String(settings?.asr_model || ""));
    setLlmBaseUrl(String(settings?.llm_base_url || ""));
    setLlmModel(String(settings?.llm_model || ""));
    setReasoning(String(settings?.llm_reasoning_effort || "default") || "default");
    setRewriteEnabled(settings?.rewrite_enabled === true);
    setRewriteTemplateId(String(settings?.rewrite_template_id || ""));
  }, [settings]);

  useEffect(() => {
    (async () => {
      await refreshModelStatus();
      await refreshTemplates();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const tpl = templates.find((t) => t.id === tplId);
    if (tpl) setTplDraft(tpl.system_prompt || "");
  }, [tplId, templates]);

  const templateOptions: PixelSelectOption[] = useMemo(() => {
    return templates.map((t) => ({ value: t.id, label: t.name }));
  }, [templates]);

  const selectedRewriteLabel = useMemo(() => {
    const t = templates.find((x) => x.id === rewriteTemplateId);
    return t?.name || "";
  }, [rewriteTemplateId, templates]);

  async function refreshTemplates() {
    try {
      const t = (await invoke("list_templates")) as PromptTemplate[];
      setTemplates(t);
      if (!tplId) setTplId(t[0]?.id || "");

      // Keep rewrite template id valid when templates list changes.
      if (rewriteTemplateId && !t.some((x) => x.id === rewriteTemplateId)) {
        setRewriteTemplateId("");
      }
    } catch {
      // templates are optional
      setTemplates([]);
    }
  }

  async function refreshModelStatus() {
    try {
      const st = (await invoke("asr_model_status")) as ModelStatus;
      setModelStatus(st);
    } catch {
      setModelStatus(null);
    }
  }

  async function downloadModel() {
    pushToast("DOWNLOADING...", "default");
    try {
      const st = (await invoke("download_asr_model")) as ModelStatus;
      setModelStatus(st);
      pushToast(st.ok ? "MODEL OK" : "MODEL FAILED", st.ok ? "ok" : "danger");
    } catch {
      pushToast("MODEL FAILED", "danger");
    }
  }

  async function saveAsr() {
    try {
      await savePatch({ asr_model: asrModel.trim() ? asrModel.trim() : null });
      pushToast("SAVED", "ok");
      await refreshModelStatus();
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveLlm() {
    try {
      await savePatch({
        llm_base_url: llmBaseUrl.trim() ? llmBaseUrl.trim() : null,
        llm_model: llmModel.trim() ? llmModel.trim() : null,
        llm_reasoning_effort: reasoning === "default" ? null : reasoning,
      });
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveRewrite() {
    try {
      await savePatch({
        rewrite_enabled: rewriteEnabled,
        rewrite_template_id: rewriteTemplateId.trim() ? rewriteTemplateId.trim() : null,
      });
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveTemplate() {
    if (!tplId) return;
    const base = templates.find((t) => t.id === tplId);
    if (!base) return;
    try {
      const updated = (await invoke("upsert_template", {
        tpl: { ...base, system_prompt: tplDraft },
      })) as PromptTemplate;
      setTemplates((prev) => prev.map((x) => (x.id === updated.id ? updated : x)));
      pushToast("TEMPLATE SAVED", "ok");
    } catch {
      pushToast("TEMPLATE SAVE FAILED", "danger");
    }
  }

  async function exportTemplates() {
    try {
      const s = (await invoke("templates_export_json")) as string;
      setTemplatesJson(s);
      pushToast("EXPORTED", "ok");
    } catch {
      pushToast("EXPORT FAILED", "danger");
    }
  }

  async function importTemplates(mode: "merge" | "replace") {
    const json = templatesJson.trim();
    if (!json) return;
    try {
      await invoke("templates_import_json", { json, mode });
      await refreshTemplates();
      pushToast("IMPORTED", "ok");
    } catch {
      pushToast("IMPORT FAILED", "danger");
    }
  }

  async function setApiKey() {
    const k = keyDraft.trim();
    if (!k) return;
    try {
      await invoke("set_llm_api_key", { apiKey: k });
      setKeyDraft("");
      pushToast("KEY SAVED", "ok");
    } catch {
      pushToast("KEY SAVE FAILED", "danger");
    }
  }

  async function clearApiKey() {
    try {
      await invoke("clear_llm_api_key");
      pushToast("KEY CLEARED", "ok");
    } catch {
      pushToast("KEY CLEAR FAILED", "danger");
    }
  }

  async function checkApiKey() {
    try {
      const st = (await invoke("llm_api_key_status")) as ApiKeyStatus;
      pushToast(
        st.configured ? "KEY OK" : "KEY MISSING",
        st.configured ? "ok" : "danger",
      );
    } catch {
      pushToast("KEY CHECK FAILED", "danger");
    }
  }

  async function clearHistory() {
    try {
      await invoke("history_clear");
      pushToast("HISTORY CLEARED", "ok");
      onHistoryCleared();
    } catch {
      pushToast("CLEAR FAILED", "danger");
    } finally {
      setConfirmClear(false);
    }
  }

  return (
    <div className="stack">
      <div className="card">
        <div className="sectionTitle">ASR</div>
        <div className="row">
          <PixelButton onClick={downloadModel} tone="accent">
            DOWNLOAD
          </PixelButton>
          <PixelButton onClick={refreshModelStatus}>REFRESH</PixelButton>
          <div className="muted">
            {modelStatus
              ? modelStatus.ok
                ? `OK  ${modelStatus.model_dir}`
                : `MISSING  ${modelStatus.reason || ""}`
              : "UNKNOWN"}
          </div>
        </div>
        <div style={{ marginTop: 12 }} className="stack">
          <PixelInput
            value={asrModel}
            onChange={setAsrModel}
            placeholder="asr_model (local dir or HF repo id)"
          />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveAsr} tone="accent">
              SAVE
            </PixelButton>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="sectionTitle">LLM</div>
        <div className="stack">
          <PixelInput
            value={llmBaseUrl}
            onChange={setLlmBaseUrl}
            placeholder="API Base URL (e.g. https://api.openai.com/v1)"
          />
          <PixelInput value={llmModel} onChange={setLlmModel} placeholder="Model" />
          <PixelSelect value={reasoning} onChange={setReasoning} options={REASONING} />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveLlm} tone="accent">
              SAVE
            </PixelButton>
          </div>
        </div>

        <div className="sectionTitle" style={{ marginTop: 18 }}>
          API KEY
        </div>
        <div className="stack">
          <PixelInput
            value={keyDraft}
            onChange={setKeyDraft}
            placeholder="save to keyring (or env TYPEVOICE_LLM_API_KEY)"
          />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={setApiKey} tone="accent" disabled={!keyDraft.trim()}>
              SAVE
            </PixelButton>
            <PixelButton onClick={clearApiKey} tone="danger">
              CLEAR
            </PixelButton>
            <PixelButton onClick={checkApiKey}>CHECK</PixelButton>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="sectionTitle">REWRITE</div>
        <div className="row" style={{ justifyContent: "space-between" }}>
          <div className="muted">
            {rewriteEnabled ? "ON" : "OFF"}
            {rewriteEnabled && selectedRewriteLabel ? `  /  ${selectedRewriteLabel}` : ""}
          </div>
          <PixelToggle value={rewriteEnabled} onChange={setRewriteEnabled} label="rewrite" />
        </div>
        <div style={{ marginTop: 12 }} className="stack">
          <PixelSelect
            value={rewriteTemplateId}
            onChange={setRewriteTemplateId}
            options={[{ value: "", label: "- template -" }, ...templateOptions]}
            disabled={!templates.length}
          />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveRewrite} tone="accent">
              SAVE
            </PixelButton>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="sectionTitle">TEMPLATES</div>
        <div className="stack">
          <PixelSelect
            value={tplId}
            onChange={setTplId}
            options={templateOptions}
            placeholder="-"
            disabled={!templates.length}
          />
          <PixelTextarea
            value={tplDraft}
            onChange={setTplDraft}
            placeholder="system prompt..."
            rows={8}
            disabled={!tplId}
          />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveTemplate} tone="accent" disabled={!tplId}>
              SAVE
            </PixelButton>
          </div>
        </div>

        <div className="sectionTitle" style={{ marginTop: 18 }}>
          IMPORT / EXPORT
        </div>
        <div className="stack">
          <PixelTextarea
            value={templatesJson}
            onChange={setTemplatesJson}
            placeholder='[{"id","name","system_prompt"}, ...]'
            rows={7}
          />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={exportTemplates}>EXPORT</PixelButton>
            <PixelButton
              onClick={() => importTemplates("merge")}
              tone="accent"
              disabled={!templatesJson.trim()}
            >
              IMPORT MERGE
            </PixelButton>
            <PixelButton
              onClick={() => importTemplates("replace")}
              tone="danger"
              disabled={!templatesJson.trim()}
            >
              IMPORT REPLACE
            </PixelButton>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="sectionTitle">HISTORY</div>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <PixelButton onClick={() => setConfirmClear(true)} tone="danger">
            CLEAR ALL
          </PixelButton>
        </div>
      </div>

      <PixelDialog
        open={confirmClear}
        title="CLEAR HISTORY"
        onClose={() => setConfirmClear(false)}
        actions={
          <>
            <PixelButton onClick={() => setConfirmClear(false)}>CANCEL</PixelButton>
            <PixelButton onClick={clearHistory} tone="danger">
              CLEAR
            </PixelButton>
          </>
        }
      >
        <div className="stack">
          <div>THIS WILL DELETE ALL HISTORY ITEMS.</div>
          <div className="muted">THIS ACTION CANNOT BE UNDONE.</div>
        </div>
      </PixelDialog>
    </div>
  );
}
