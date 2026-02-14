import { useEffect, useMemo, useState } from "react";
import { defaultTauriGateway, type TauriGateway } from "../infra/runtimePorts";
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
  gateway?: TauriGateway;
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

type RecordingHotkey = "ptt" | "toggle";

type HotkeyAvailability = {
  available: boolean;
  reason?: string | null;
  reason_code?: string | null;
};

const MODIFIER_ONLY_KEY_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

function isModifierOnlyCode(code: string): boolean {
  return MODIFIER_ONLY_KEY_CODES.has(code);
}

function shortcutFromKeyboardEvent(event: KeyboardEvent): string | null {
  if (isModifierOnlyCode(event.code)) {
    return null;
  }

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("CTRL");
  if (event.shiftKey) parts.push("SHIFT");
  if (event.altKey) parts.push("ALT");
  if (event.metaKey) parts.push("SUPER");
  const base = event.code.trim();
  if (!base) return null;
  parts.push(base.toUpperCase());
  return parts.join("+");
}

function isSameHotkey(a: string, b: string): boolean {
  return a.trim().toUpperCase() === b.trim().toUpperCase();
}

export function SettingsScreen({
  settings,
  savePatch,
  pushToast,
  onHistoryCleared,
  gateway = defaultTauriGateway,
}: Props) {
  const [asrModel, setAsrModel] = useState("");
  const [asrPreprocessTrimEnabled, setAsrPreprocessTrimEnabled] = useState(false);
  const [asrPreprocessThresholdDb, setAsrPreprocessThresholdDb] = useState("-50");
  const [asrPreprocessStartMs, setAsrPreprocessStartMs] = useState("300");
  const [asrPreprocessEndMs, setAsrPreprocessEndMs] = useState("300");
  const [llmBaseUrl, setLlmBaseUrl] = useState("");
  const [llmModel, setLlmModel] = useState("");
  const [reasoning, setReasoning] = useState("default");
  const [rewriteEnabled, setRewriteEnabled] = useState(false);
  const [rewriteTemplateId, setRewriteTemplateId] = useState("");
  const [rewriteGlossaryDraft, setRewriteGlossaryDraft] = useState("");

  const [hotkeysEnabled, setHotkeysEnabled] = useState(true);
  const [hotkeyPtt, setHotkeyPtt] = useState("F9");
  const [hotkeyToggle, setHotkeyToggle] = useState("F10");
  const [recordingHotkey, setRecordingHotkey] = useState<RecordingHotkey | null>(null);
  const [hotkeysShowOverlay, setHotkeysShowOverlay] = useState(true);
  const [contextIncludeHistory, setContextIncludeHistory] = useState(true);
  const [contextIncludeClipboard, setContextIncludeClipboard] = useState(true);
  const [contextIncludePrevWindowMeta, setContextIncludePrevWindowMeta] = useState(true);
  const [contextIncludePrevWindowScreenshot, setContextIncludePrevWindowScreenshot] =
    useState(true);
  const [rewriteIncludeGlossary, setRewriteIncludeGlossary] = useState(true);

  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);

  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  const [tplId, setTplId] = useState("");
  const [tplDraft, setTplDraft] = useState("");

  const [keyDraft, setKeyDraft] = useState("");
  const [templatesJson, setTemplatesJson] = useState("");

  const [confirmClear, setConfirmClear] = useState(false);
  const [isCheckingHotkey, setIsCheckingHotkey] = useState(false);

  useEffect(() => {
    if (!settings) return;
    setAsrModel(settings.asr_model ?? "");
    setAsrPreprocessTrimEnabled(settings.asr_preprocess_silence_trim_enabled ?? false);
    setAsrPreprocessThresholdDb(
      String(
        settings.asr_preprocess_silence_threshold_db ??
          -50,
      ),
    );
    setAsrPreprocessStartMs(
      String(
        settings.asr_preprocess_silence_start_ms ??
          300,
      ),
    );
    setAsrPreprocessEndMs(String(settings.asr_preprocess_silence_end_ms ?? 300));
    setLlmBaseUrl(settings.llm_base_url ?? "");
    setLlmModel(settings.llm_model ?? "");
    setReasoning(settings.llm_reasoning_effort ?? "default");

    if (typeof settings.rewrite_enabled !== "boolean") {
      pushToast("SETTINGS INVALID: rewrite_enabled missing", "danger");
      return;
    }
    setRewriteEnabled(settings.rewrite_enabled);
    setRewriteTemplateId(settings.rewrite_template_id ?? "");
    setRewriteGlossaryDraft((settings.rewrite_glossary || []).join("\n"));
    setRewriteIncludeGlossary(settings.rewrite_include_glossary ?? true);

    if (typeof settings.hotkeys_enabled !== "boolean") {
      pushToast("SETTINGS INVALID: hotkeys_enabled missing", "danger");
      return;
    }
    if (typeof settings.hotkeys_show_overlay !== "boolean") {
      pushToast("SETTINGS INVALID: hotkeys_show_overlay missing", "danger");
      return;
    }
    setHotkeysEnabled(settings.hotkeys_enabled);
    setHotkeyPtt(settings.hotkey_ptt ?? "");
    setHotkeyToggle(settings.hotkey_toggle ?? "");
    setHotkeysShowOverlay(settings.hotkeys_show_overlay);

    setContextIncludeHistory(settings.context_include_history ?? true);
    setContextIncludeClipboard(settings.context_include_clipboard ?? true);
    setContextIncludePrevWindowMeta(settings.context_include_prev_window_meta ?? true);
    setContextIncludePrevWindowScreenshot(
      settings.context_include_prev_window_screenshot ?? true,
    );
  }, [settings, pushToast]);

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

  async function startRecording(target: RecordingHotkey) {
    if (!hotkeysEnabled) return;
    if (recordingHotkey === target) {
      setRecordingHotkey(null);
      return;
    }
    setRecordingHotkey(target);
    pushToast("PRESS A KEYBOARD SHORTCUT", "default");
  }

  async function refreshTemplates() {
    try {
      const t = (await gateway.invoke("list_templates")) as PromptTemplate[];
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
      const st = (await gateway.invoke("asr_model_status")) as ModelStatus;
      setModelStatus(st);
    } catch {
      setModelStatus(null);
    }
  }

  async function downloadModel() {
    pushToast("DOWNLOADING...", "default");
    try {
      const st = (await gateway.invoke("download_asr_model")) as ModelStatus;
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

  async function savePreprocessConfig() {
    const thresholdDb = Number(asrPreprocessThresholdDb);
    const trimStartMs = Number(asrPreprocessStartMs);
    const trimEndMs = Number(asrPreprocessEndMs);
    if (!Number.isFinite(thresholdDb) || !Number.isFinite(trimStartMs) || !Number.isFinite(trimEndMs)) {
      pushToast("INVALID PREPROCESS INPUT", "danger");
      return;
    }
    if (thresholdDb > 0) {
      pushToast("SILENCE THRESHOLD SHOULD NOT BE ABOVE 0", "danger");
      return;
    }
    if (trimStartMs < 0 || trimEndMs < 0) {
      pushToast("TRIM MS SHOULD BE >= 0", "danger");
      return;
    }
    try {
      await savePatch({
        asr_preprocess_silence_trim_enabled: asrPreprocessTrimEnabled,
        asr_preprocess_silence_threshold_db: thresholdDb,
        asr_preprocess_silence_start_ms: Number.isInteger(trimStartMs)
          ? trimStartMs
          : Math.round(trimStartMs),
        asr_preprocess_silence_end_ms: Number.isInteger(trimEndMs)
          ? trimEndMs
          : Math.round(trimEndMs),
      });
      pushToast("SAVED", "ok");
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
    if (rewriteEnabled && !rewriteTemplateId.trim()) {
      pushToast("REWRITE TEMPLATE REQUIRED", "danger");
      return;
    }
    try {
      await savePatch({
        rewrite_enabled: rewriteEnabled,
        rewrite_template_id: rewriteTemplateId.trim() ? rewriteTemplateId.trim() : null,
        rewrite_include_glossary: rewriteIncludeGlossary,
      });
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveGlossary() {
    const items = rewriteGlossaryDraft
      .split("\n")
      .map((x) => x.trim())
      .filter((x) => x.length > 0);
    try {
      await savePatch({ rewrite_glossary: items });
      pushToast("GLOSSARY SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveContextConfig() {
    try {
      await savePatch({
        context_include_history: contextIncludeHistory,
        context_include_clipboard: contextIncludeClipboard,
        context_include_prev_window_meta: contextIncludePrevWindowMeta,
        context_include_prev_window_screenshot: contextIncludePrevWindowScreenshot,
      });
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveHotkeys() {
    const ptt = hotkeyPtt.trim();
    const toggle = hotkeyToggle.trim();
    if (hotkeysEnabled && (!ptt || !toggle)) {
      pushToast("HOTKEYS REQUIRE PTT/TOGGLE", "danger");
      return;
    }
    if (hotkeysEnabled && isSameHotkey(ptt, toggle)) {
      pushToast("HOTKEYS PTT and TOGGLE cannot be the same", "danger");
      return;
    }
    try {
      await savePatch({
        hotkeys_enabled: hotkeysEnabled,
        hotkey_ptt: ptt || null,
        hotkey_toggle: toggle || null,
        hotkeys_show_overlay: hotkeysShowOverlay,
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
      const updated = (await gateway.invoke("upsert_template", {
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
      const s = (await gateway.invoke("templates_export_json")) as string;
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
      await gateway.invoke("templates_import_json", { json, mode });
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
      await gateway.invoke("set_llm_api_key", { apiKey: k });
      setKeyDraft("");
      pushToast("KEY SAVED", "ok");
    } catch {
      pushToast("KEY SAVE FAILED", "danger");
    }
  }

  async function clearApiKey() {
    try {
      await gateway.invoke("clear_llm_api_key");
      pushToast("KEY CLEARED", "ok");
    } catch {
      pushToast("KEY CLEAR FAILED", "danger");
    }
  }

  async function checkApiKey() {
    try {
      const st = (await gateway.invoke("llm_api_key_status")) as ApiKeyStatus;
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
      await gateway.invoke("history_clear");
      pushToast("HISTORY CLEARED", "ok");
      onHistoryCleared();
    } catch {
      pushToast("CLEAR FAILED", "danger");
    } finally {
      setConfirmClear(false);
    }
  }

  useEffect(() => {
    if (!recordingHotkey) return;
    if (!hotkeysEnabled) {
      setRecordingHotkey(null);
      return;
    }

    const onKeyDown = async (event: KeyboardEvent) => {
      if (event.repeat) return;
      event.preventDefault();

      const candidate = shortcutFromKeyboardEvent(event);
      if (!candidate) {
        pushToast("HOTKEY NOT AVAILABLE: modifier-only keys are not valid", "danger");
        return;
      }

      const otherKey = recordingHotkey === "ptt" ? hotkeyToggle : hotkeyPtt;
      if (otherKey && isSameHotkey(candidate, otherKey)) {
        pushToast("HOTKEY NOT AVAILABLE: PTT and TOGGLE must be different", "danger");
        return;
      }

      setIsCheckingHotkey(true);

      try {
        const ignoreSelf = recordingHotkey === "ptt" ? hotkeyPtt : hotkeyToggle;
        const result = (await gateway.invoke("check_hotkey_available", {
          shortcut: candidate,
          ignore_self: ignoreSelf?.trim() || null,
        })) as HotkeyAvailability;

        if (!result.available) {
          const code = result.reason_code ? ` (${result.reason_code})` : "";
          pushToast(
            `HOTKEY NOT AVAILABLE: ${result.reason || "Unavailable"}${code}`,
            "danger",
          );
          return;
        }

        const normalized = candidate.toUpperCase();
        if (recordingHotkey === "ptt") {
          setHotkeyPtt(normalized);
        } else {
          setHotkeyToggle(normalized);
        }
        setRecordingHotkey(null);
      } catch {
        pushToast("HOTKEY CHECK FAILED", "danger");
      } finally {
        setIsCheckingHotkey(false);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [
    recordingHotkey,
    hotkeysEnabled,
    hotkeyPtt,
    hotkeyToggle,
    pushToast,
    gateway,
  ]);

  const asrStatusText = useMemo(() => {
    if (!modelStatus) return "UNKNOWN";

    const version = modelStatus.model_version ? `  ${modelStatus.model_version}` : "";
    const location = `  ${modelStatus.model_dir}`;

    if (modelStatus.ok) {
      switch (modelStatus.reason) {
        case "manifest.json_missing":
          return `OK${version}  manifest.json_missing (integrity checks skipped, ASR still usable)${location}`;
        case "remote_model_not_locally_verified":
          return `OK${version}  remote model id, not locally verified${location}`;
        case null:
        case undefined:
          return `OK${version}${location}`;
        default:
          return `OK${version}  ${modelStatus.reason}${location}`;
      }
    }

    return `FAILED${version}  ${modelStatus.reason || ""}${location}`;
  }, [modelStatus]);

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
            {asrStatusText}
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
        <div className="sectionTitle" style={{ marginTop: 18 }}>
          PREPROCESS
        </div>
        <div className="stack">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">{asrPreprocessTrimEnabled ? "SILENCE TRIM ON" : "SILENCE TRIM OFF"}</div>
            <PixelToggle
              value={asrPreprocessTrimEnabled}
              onChange={setAsrPreprocessTrimEnabled}
              label="silence trim"
            />
          </div>
          <div className="row">
            <div className="muted">阈值（dB）</div>
            <PixelInput
              value={asrPreprocessThresholdDb}
              onChange={setAsrPreprocessThresholdDb}
              placeholder="-50"
            />
          </div>
          <div className="row">
            <div className="muted">前段静音 (ms)</div>
            <PixelInput
              value={asrPreprocessStartMs}
              onChange={setAsrPreprocessStartMs}
              placeholder="300"
            />
          </div>
          <div className="row">
            <div className="muted">末段静音 (ms)</div>
            <PixelInput
              value={asrPreprocessEndMs}
              onChange={setAsrPreprocessEndMs}
              placeholder="300"
            />
          </div>
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={savePreprocessConfig} tone="accent">
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
        <div className="sectionTitle">REWRITE CONTEXT SWITCH</div>
        <div className="stack">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">Context 历史片段</div>
            <PixelToggle
              value={contextIncludeHistory}
              onChange={setContextIncludeHistory}
              label="history"
            />
          </div>
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">Context 剪贴板</div>
            <PixelToggle
              value={contextIncludeClipboard}
              onChange={setContextIncludeClipboard}
              label="clipboard"
            />
          </div>
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">Window 元信息</div>
            <PixelToggle
              value={contextIncludePrevWindowMeta}
              onChange={setContextIncludePrevWindowMeta}
              label="prev window meta"
            />
          </div>
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">Window 截图</div>
            <PixelToggle
              value={contextIncludePrevWindowScreenshot}
              onChange={setContextIncludePrevWindowScreenshot}
              label="prev window screenshot"
            />
          </div>
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveContextConfig} tone="accent">
              SAVE
            </PixelButton>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="sectionTitle">GLOSSARY</div>
        <div className="muted">
          每行一个词；空行会自动忽略。用于 rewrite 阶段作为“上下文词汇/术语”约束模型遵循。
        </div>
        <div style={{ marginTop: 12 }} className="stack">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">REWRITE 词库启用</div>
            <PixelToggle
              value={rewriteIncludeGlossary}
              onChange={setRewriteIncludeGlossary}
              label="rewrite glossary"
            />
          </div>
          <PixelTextarea
            value={rewriteGlossaryDraft}
            onChange={setRewriteGlossaryDraft}
            placeholder={"比如：QPSK\nTypeScript\nOAuth"}
            rows={8}
          />
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveGlossary} tone="accent">
              SAVE
            </PixelButton>
          </div>
        </div>
      </div>

      <div className="card">
        <div className="sectionTitle">HOTKEYS</div>
        <div className="row" style={{ justifyContent: "space-between" }}>
          <div className="muted">
            {hotkeysEnabled ? "ON" : "OFF"}
            {hotkeysEnabled ? `  /  PTT ${hotkeyPtt || "-"}  /  TOGGLE ${hotkeyToggle || "-"}` : ""}
          </div>
          <PixelToggle value={hotkeysEnabled} onChange={setHotkeysEnabled} label="hotkeys" />
        </div>
        <div style={{ marginTop: 12 }} className="stack">
          <div className="row">
            <PixelInput
              value={hotkeyPtt}
              onChange={setHotkeyPtt}
              placeholder="PTT (press to talk) e.g. F9"
              disabled={!hotkeysEnabled}
              readOnly
            />
            <PixelButton
              onClick={() => startRecording("ptt")}
              disabled={!hotkeysEnabled || (recordingHotkey !== null && recordingHotkey !== "ptt")}
              tone={recordingHotkey === "ptt" ? "accent" : "default"}
            >
              {recordingHotkey === "ptt"
                ? isCheckingHotkey
                  ? "CHECKING..."
                  : "WAITING"
                : "RECORD"}
            </PixelButton>
          </div>
          <div className="row">
            <PixelInput
              value={hotkeyToggle}
              onChange={setHotkeyToggle}
              placeholder="TOGGLE e.g. F10"
              disabled={!hotkeysEnabled}
              readOnly
            />
            <PixelButton
              onClick={() => startRecording("toggle")}
              disabled={!hotkeysEnabled || (recordingHotkey !== null && recordingHotkey !== "toggle")}
              tone={recordingHotkey === "toggle" ? "accent" : "default"}
            >
              {recordingHotkey === "toggle"
                ? isCheckingHotkey
                  ? "CHECKING..."
                  : "WAITING"
                : "RECORD"}
            </PixelButton>
          </div>
          <div className="row" style={{ justifyContent: "space-between" }}>
            <div className="muted">{hotkeysShowOverlay ? "OVERLAY ON" : "OVERLAY OFF"}</div>
            <PixelToggle
              value={hotkeysShowOverlay}
              onChange={setHotkeysShowOverlay}
              label="overlay"
            />
          </div>
          <div className="row" style={{ justifyContent: "flex-end" }}>
            <PixelButton onClick={saveHotkeys} tone="accent">
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
