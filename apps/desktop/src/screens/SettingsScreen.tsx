import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import { defaultTauriGateway, type TauriGateway } from "../infra/runtimePorts";
import type {
  ApiCheckResult,
  AudioCaptureDevice,
  Settings,
} from "../types";
import { PixelButton } from "../ui/PixelButton";
import { PixelDialog } from "../ui/PixelDialog";
import { PixelInput, PixelTextarea } from "../ui/PixelInput";
import { PixelSelect, type PixelSelectOption } from "../ui/PixelSelect";
import { PixelToggle } from "../ui/PixelToggle";
import { IconGear } from "../ui/icons";

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

const ASR_PROVIDERS: PixelSelectOption[] = [
  { value: "doubao", label: "doubao streaming" },
  { value: "remote", label: "remote (cloud)" },
];

const RECORD_INPUT_STRATEGIES: PixelSelectOption[] = [
  { value: "follow_default", label: "follow system default" },
  { value: "fixed_device", label: "fixed specific device" },
  { value: "auto_select", label: "auto-select available" },
];

const RECORD_DEFAULT_ROLES: PixelSelectOption[] = [
  { value: "communications", label: "communications (eCommunications)" },
  { value: "console", label: "console (eConsole)" },
];

type SettingsPanelId =
  | "asr"
  | "recording"
  | "preprocess"
  | "llm"
  | "llmKey"
  | "rewrite"
  | "context"
  | "glossary"
  | "export"
  | "hotkeys"
  | "history";

type SettingsLineProps = {
  title: string;
  detail?: string;
  panel: SettingsPanelId;
  activePanel: SettingsPanelId | null;
  onTogglePanel: (panel: SettingsPanelId) => void;
  control?: ReactNode;
  children?: ReactNode;
};

function SettingsLine({
  title,
  detail,
  panel,
  activePanel,
  onTogglePanel,
  control,
  children,
}: SettingsLineProps) {
  const expanded = activePanel === panel;
  return (
    <div className={`settingsLineBlock ${expanded ? "isExpanded" : ""}`}>
      <div className="settingsLine">
        <div className="settingsLineText">
          <div className="settingsLineTitle">{title}</div>
          {detail ? <div className="settingsLineDetail">{detail}</div> : null}
        </div>
        <button
          type="button"
          className="settingsGear"
          aria-label={`${title} settings`}
          aria-expanded={expanded}
          onClick={() => onTogglePanel(panel)}
        >
          <IconGear size={22} tone={expanded ? "accent" : "muted"} filled={expanded} />
        </button>
        {control ? <div className="settingsLineControl">{control}</div> : null}
      </div>
      {expanded && children ? <div className="settingsLinePanel">{children}</div> : null}
    </div>
  );
}

export function SettingsScreen({
  settings,
  savePatch,
  pushToast,
  onHistoryCleared,
  gateway = defaultTauriGateway,
}: Props) {
  const [asrProvider, setAsrProvider] = useState("doubao");
  const [remoteAsrUrl, setRemoteAsrUrl] = useState("https://api.server/transcribe");
  const [remoteAsrModel, setRemoteAsrModel] = useState("");
  const [remoteAsrConcurrency, setRemoteAsrConcurrency] = useState("4");
  const [remoteAsrKeyDraft, setRemoteAsrKeyDraft] = useState("");
  const [doubaoAppKeyDraft, setDoubaoAppKeyDraft] = useState("");
  const [doubaoAccessKeyDraft, setDoubaoAccessKeyDraft] = useState("");
  const [asrPreprocessTrimEnabled, setAsrPreprocessTrimEnabled] = useState(false);
  const [asrPreprocessThresholdDb, setAsrPreprocessThresholdDb] = useState("-50");
  const [asrPreprocessStartMs, setAsrPreprocessStartMs] = useState("300");
  const [asrPreprocessEndMs, setAsrPreprocessEndMs] = useState("300");
  const [llmBaseUrl, setLlmBaseUrl] = useState("");
  const [llmModel, setLlmModel] = useState("");
  const [reasoning, setReasoning] = useState("default");
  const [llmPrompt, setLlmPrompt] = useState("");
  const [rewriteEnabled, setRewriteEnabled] = useState(false);
  const [rewriteGlossaryDraft, setRewriteGlossaryDraft] = useState("");
  const [autoPasteEnabled, setAutoPasteEnabled] = useState(true);
  const [recordInputStrategy, setRecordInputStrategy] = useState("follow_default");
  const [recordFollowDefaultRole, setRecordFollowDefaultRole] = useState("communications");
  const [recordFixedEndpointId, setRecordFixedEndpointId] = useState("");
  const [recordFixedFriendlyName, setRecordFixedFriendlyName] = useState("");
  const [audioCaptureDevices, setAudioCaptureDevices] = useState<AudioCaptureDevice[]>([]);

  const [hotkeysEnabled, setHotkeysEnabled] = useState(true);
  const [hotkeysShowOverlay, setHotkeysShowOverlay] = useState(true);
  const [contextIncludeHistory, setContextIncludeHistory] = useState(true);
  const [contextIncludeClipboard, setContextIncludeClipboard] = useState(true);
  const [contextIncludePrevWindowMeta, setContextIncludePrevWindowMeta] = useState(true);
  const [contextIncludePrevWindowScreenshot, setContextIncludePrevWindowScreenshot] =
    useState(true);
  const [rewriteIncludeGlossary, setRewriteIncludeGlossary] = useState(true);

  const [keyDraft, setKeyDraft] = useState("");

  const [confirmClear, setConfirmClear] = useState(false);
  const [llmCheckPending, setLlmCheckPending] = useState(false);
  const [remoteAsrCheckPending, setRemoteAsrCheckPending] = useState(false);
  const [doubaoCheckPending, setDoubaoCheckPending] = useState(false);
  const [activeSettingsPanel, setActiveSettingsPanel] = useState<SettingsPanelId | null>(null);

  useEffect(() => {
    if (!settings) return;
    setAsrProvider(
      settings.asr_provider === "remote"
        ? "remote"
        : "doubao",
    );
    setRemoteAsrUrl(settings.remote_asr_url?.trim() || "https://api.server/transcribe");
    setRemoteAsrModel(settings.remote_asr_model ?? "");
    {
      const raw = Number(settings.remote_asr_concurrency ?? 4);
      const normalized = Number.isFinite(raw) ? Math.max(1, Math.min(16, Math.round(raw))) : 4;
      setRemoteAsrConcurrency(String(normalized));
    }
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
    setLlmPrompt(settings.llm_prompt ?? "");

    if (typeof settings.rewrite_enabled !== "boolean") {
      pushToast("Settings need attention", "danger");
      return;
    }
    setRewriteEnabled(settings.rewrite_enabled);
    setRewriteGlossaryDraft((settings.rewrite_glossary || []).join("\n"));
    setRewriteIncludeGlossary(settings.rewrite_include_glossary ?? true);
    setAutoPasteEnabled(settings.auto_paste_enabled ?? true);
    setRecordInputStrategy(
      settings.record_input_strategy === "fixed_device"
        ? "fixed_device"
        : settings.record_input_strategy === "auto_select"
          ? "auto_select"
          : "follow_default",
    );
    setRecordFollowDefaultRole(
      settings.record_follow_default_role === "console" ? "console" : "communications",
    );
    setRecordFixedEndpointId(settings.record_fixed_endpoint_id ?? "");
    setRecordFixedFriendlyName(settings.record_fixed_friendly_name ?? "");

    if (typeof settings.hotkeys_enabled !== "boolean") {
      pushToast("Settings need attention", "danger");
      return;
    }
    if (typeof settings.hotkeys_show_overlay !== "boolean") {
      pushToast("Settings need attention", "danger");
      return;
    }
    setHotkeysEnabled(settings.hotkeys_enabled);
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
      await refreshAudioCaptureDevices();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const captureDeviceOptions: PixelSelectOption[] = useMemo(() => {
    return audioCaptureDevices.map((v) => {
      let label = v.friendly_name;
      if (v.is_default_communications) {
        label += " [default communications]";
      }
      if (v.is_default_console) {
        label += " [default console]";
      }
      return { value: v.endpoint_id, label };
    });
  }, [audioCaptureDevices]);

  useEffect(() => {
    const found = audioCaptureDevices.find((v) => v.endpoint_id === recordFixedEndpointId);
    if (!found) return;
    setRecordFixedFriendlyName(found.friendly_name);
  }, [audioCaptureDevices, recordFixedEndpointId]);

  async function refreshAudioCaptureDevices() {
    try {
      const rows = (await gateway.invoke(
        "list_audio_capture_devices",
      )) as AudioCaptureDevice[];
      setAudioCaptureDevices(rows);
      if (!recordFixedEndpointId.trim()) return;
      const found = rows.find((v) => v.endpoint_id === recordFixedEndpointId);
      if (!found) return;
      setRecordFixedFriendlyName(found.friendly_name);
    } catch {
      setAudioCaptureDevices([]);
    }
  }

  async function saveAsr() {
    const provider = asrProvider === "remote" ? "remote" : "doubao";
    const concurrencyNum = Number(remoteAsrConcurrency);
    if (provider === "remote" && !remoteAsrUrl.trim()) {
      pushToast("REMOTE ASR URL REQUIRED", "danger");
      return;
    }
    if (!Number.isFinite(concurrencyNum)) {
      pushToast("REMOTE ASR CONCURRENCY MUST BE A NUMBER", "danger");
      return;
    }
    const normalizedConcurrency = Math.max(1, Math.min(16, Math.round(concurrencyNum)));
    try {
      await savePatch({
        asr_provider: provider,
        remote_asr_url: remoteAsrUrl.trim() ? remoteAsrUrl.trim() : null,
        remote_asr_model: remoteAsrModel.trim() ? remoteAsrModel.trim() : null,
        remote_asr_concurrency: normalizedConcurrency,
      });
      setRemoteAsrConcurrency(String(normalizedConcurrency));
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveRecordingInput() {
    const strategy =
      recordInputStrategy === "fixed_device"
        ? "fixed_device"
        : recordInputStrategy === "auto_select"
          ? "auto_select"
          : "follow_default";
    const role = recordFollowDefaultRole === "console" ? "console" : "communications";
    if (strategy === "fixed_device" && !recordFixedEndpointId.trim()) {
      pushToast("FIXED DEVICE MUST BE SELECTED", "danger");
      return;
    }
    const selected = audioCaptureDevices.find(
      (v) => v.endpoint_id === recordFixedEndpointId.trim(),
    );
    try {
      await savePatch({
        record_input_strategy: strategy,
        record_follow_default_role: role,
        record_fixed_endpoint_id:
          strategy === "fixed_device" ? recordFixedEndpointId.trim() : null,
        record_fixed_friendly_name:
          strategy === "fixed_device"
            ? (selected?.friendly_name || recordFixedFriendlyName || "").trim() || null
            : null,
      });
      if (selected) {
        setRecordFixedFriendlyName(selected.friendly_name);
      }
      pushToast("SAVED", "ok");
      await refreshAudioCaptureDevices();
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
    if (rewriteEnabled && !llmPrompt.trim()) {
      pushToast("LLM PROMPT REQUIRED", "danger");
      return;
    }
    try {
      await savePatch({
        rewrite_enabled: rewriteEnabled,
        llm_prompt: llmPrompt,
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

  async function saveExportConfig() {
    try {
      await savePatch({
        auto_paste_enabled: autoPasteEnabled,
      });
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
    }
  }

  async function saveHotkeys() {
    try {
      await savePatch({
        hotkeys_enabled: hotkeysEnabled,
        hotkey_ptt: null,
        hotkey_toggle: null,
        hotkeys_show_overlay: hotkeysShowOverlay,
      });
      pushToast("SAVED", "ok");
    } catch {
      pushToast("SAVE FAILED", "danger");
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
    if (llmCheckPending) return;
    setLlmCheckPending(true);
    try {
      const result = (await gateway.invoke("check_llm_api_key", {
        baseUrl: llmBaseUrl,
        model: llmModel,
        reasoningEffort: reasoning,
      })) as ApiCheckResult;
      pushToast(result.message, result.ok ? "ok" : "danger");
    } catch {
      pushToast("API key check failed. Try again after checking the settings.", "danger");
    } finally {
      setLlmCheckPending(false);
    }
  }

  async function setRemoteAsrApiKey() {
    const k = remoteAsrKeyDraft.trim();
    if (!k) return;
    try {
      await gateway.invoke("set_remote_asr_api_key", { apiKey: k });
      setRemoteAsrKeyDraft("");
      pushToast("REMOTE KEY SAVED", "ok");
    } catch {
      pushToast("REMOTE KEY SAVE FAILED", "danger");
    }
  }

  async function clearRemoteAsrApiKey() {
    try {
      await gateway.invoke("clear_remote_asr_api_key");
      pushToast("REMOTE KEY CLEARED", "ok");
    } catch {
      pushToast("REMOTE KEY CLEAR FAILED", "danger");
    }
  }

  async function checkRemoteAsrApiKey() {
    if (remoteAsrCheckPending) return;
    setRemoteAsrCheckPending(true);
    try {
      const result = (await gateway.invoke("check_remote_asr_api_key", {
        url: remoteAsrUrl,
        model: remoteAsrModel,
      })) as ApiCheckResult;
      pushToast(result.message, result.ok ? "ok" : "danger");
    } catch {
      pushToast("Remote ASR API check failed. Try again after checking the settings.", "danger");
    } finally {
      setRemoteAsrCheckPending(false);
    }
  }

  async function setDoubaoAsrCredentials() {
    const appKey = doubaoAppKeyDraft.trim();
    const accessKey = doubaoAccessKeyDraft.trim();
    if (!appKey || !accessKey) return;
    try {
      await gateway.invoke("set_doubao_asr_credentials", { appKey, accessKey });
      setDoubaoAppKeyDraft("");
      setDoubaoAccessKeyDraft("");
      pushToast("DOUBAO KEY SAVED", "ok");
    } catch {
      pushToast("DOUBAO KEY SAVE FAILED", "danger");
    }
  }

  async function clearDoubaoAsrCredentials() {
    try {
      await gateway.invoke("clear_doubao_asr_credentials");
      pushToast("DOUBAO KEY CLEARED", "ok");
    } catch {
      pushToast("DOUBAO KEY CLEAR FAILED", "danger");
    }
  }

  async function checkDoubaoAsrCredentials() {
    if (doubaoCheckPending) return;
    setDoubaoCheckPending(true);
    try {
      const result = (await gateway.invoke("check_doubao_asr_credentials")) as ApiCheckResult;
      pushToast(result.message, result.ok ? "ok" : "danger");
    } catch {
      pushToast("Doubao ASR API check failed. Try again after checking the settings.", "danger");
    } finally {
      setDoubaoCheckPending(false);
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

  const asrStatusText = useMemo(() => {
    if (asrProvider === "doubao") {
      return "Doubao streaming  wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";
    }
    return `Remote ${remoteAsrUrl.trim() || "https://api.server/transcribe"}`;
  }, [asrProvider, remoteAsrUrl]);

  function toggleSettingsPanel(panel: SettingsPanelId) {
    setActiveSettingsPanel((current) => current === panel ? null : panel);
  }

  return (
    <div className="pageSurface settingsSurface">
      <div className="pageHeader settingsHeader">
        <div className="sectionTitle">settings</div>
        <div className="ok">Saved</div>
      </div>
      <div className="settingsGrid">
        <div className="card">
          <SettingsLine
            title="Speech recognition"
            detail={asrStatusText}
            panel="asr"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={<PixelSelect value={asrProvider} onChange={setAsrProvider} options={ASR_PROVIDERS} />}
          >
            <div className="stack">
              {asrProvider === "doubao" ? (
                <>
                  <PixelInput
                    value={doubaoAppKeyDraft}
                    onChange={setDoubaoAppKeyDraft}
                    placeholder="App Key (or env TYPEVOICE_DOUBAO_ASR_APP_KEY)"
                  />
                  <PixelInput
                    value={doubaoAccessKeyDraft}
                    onChange={setDoubaoAccessKeyDraft}
                    placeholder="Access Key (or env TYPEVOICE_DOUBAO_ASR_ACCESS_KEY)"
                  />
                  <div className="row" style={{ justifyContent: "flex-end" }}>
                    <PixelButton
                      onClick={setDoubaoAsrCredentials}
                      tone="accent"
                      disabled={!doubaoAppKeyDraft.trim() || !doubaoAccessKeyDraft.trim()}
                    >
                      Save key
                    </PixelButton>
                    <PixelButton onClick={clearDoubaoAsrCredentials} tone="danger">
                      Clear key
                    </PixelButton>
                    <PixelButton onClick={checkDoubaoAsrCredentials} disabled={doubaoCheckPending}>
                      {doubaoCheckPending ? "Checking" : "Check key"}
                    </PixelButton>
                  </div>
                </>
              ) : (
                <>
                  <PixelInput
                    value={remoteAsrUrl}
                    onChange={setRemoteAsrUrl}
                    placeholder="remote ASR URL (e.g. https://api.server/transcribe)"
                  />
                  <PixelInput
                    value={remoteAsrModel}
                    onChange={setRemoteAsrModel}
                    placeholder="remote model name (optional)"
                  />
                  <PixelInput
                    value={remoteAsrConcurrency}
                    onChange={setRemoteAsrConcurrency}
                    placeholder="remote slicing concurrency (1-16)"
                  />
                  <PixelInput
                    value={remoteAsrKeyDraft}
                    onChange={setRemoteAsrKeyDraft}
                    placeholder="save to keyring (or env TYPEVOICE_REMOTE_ASR_API_KEY)"
                  />
                  <div className="row" style={{ justifyContent: "flex-end" }}>
                    <PixelButton
                      onClick={setRemoteAsrApiKey}
                      tone="accent"
                      disabled={!remoteAsrKeyDraft.trim()}
                    >
                      Save key
                    </PixelButton>
                    <PixelButton onClick={clearRemoteAsrApiKey} tone="danger">
                      Clear key
                    </PixelButton>
                    <PixelButton onClick={checkRemoteAsrApiKey} disabled={remoteAsrCheckPending}>
                      {remoteAsrCheckPending ? "Checking" : "Check key"}
                    </PixelButton>
                  </div>
                </>
              )}
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveAsr} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>

          <SettingsLine
            title="Recording input"
            detail={recordFixedFriendlyName || "Capture source"}
            panel="recording"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={
              <PixelSelect
                value={recordInputStrategy}
                onChange={setRecordInputStrategy}
                options={RECORD_INPUT_STRATEGIES}
              />
            }
          >
            <div className="stack">
              {recordInputStrategy === "follow_default" ? (
                <PixelSelect
                  value={recordFollowDefaultRole}
                  onChange={setRecordFollowDefaultRole}
                  options={RECORD_DEFAULT_ROLES}
                />
              ) : null}
              {recordInputStrategy === "fixed_device" ? (
                <>
                  <PixelSelect
                    value={recordFixedEndpointId}
                    onChange={setRecordFixedEndpointId}
                    options={captureDeviceOptions}
                    placeholder="select fixed capture endpoint"
                  />
                  {recordFixedFriendlyName ? (
                    <div className="muted">fixed: {recordFixedFriendlyName}</div>
                  ) : null}
                </>
              ) : null}
              {audioCaptureDevices.length === 0 ? (
                <div className="muted">No active capture endpoints detected.</div>
              ) : null}
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={refreshAudioCaptureDevices}>Refresh</PixelButton>
                <PixelButton onClick={saveRecordingInput} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>

          <SettingsLine
            title="Silence trim"
            detail={asrPreprocessTrimEnabled ? "On" : "Off"}
            panel="preprocess"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={
              <PixelToggle
                value={asrPreprocessTrimEnabled}
                onChange={setAsrPreprocessTrimEnabled}
                label="silence trim"
              />
            }
          >
            <div className="stack">
              <div className="settingsField">
                <div className="muted">阈值（dB）</div>
                <PixelInput
                  value={asrPreprocessThresholdDb}
                  onChange={setAsrPreprocessThresholdDb}
                  placeholder="-50"
                />
              </div>
              <div className="settingsField">
                <div className="muted">前段静音 (ms)</div>
                <PixelInput
                  value={asrPreprocessStartMs}
                  onChange={setAsrPreprocessStartMs}
                  placeholder="300"
                />
              </div>
              <div className="settingsField">
                <div className="muted">末段静音 (ms)</div>
                <PixelInput
                  value={asrPreprocessEndMs}
                  onChange={setAsrPreprocessEndMs}
                  placeholder="300"
                />
              </div>
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={savePreprocessConfig} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="Language model"
            detail={llmModel.trim() || "Model settings"}
            panel="llm"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={<PixelSelect value={reasoning} onChange={setReasoning} options={REASONING} />}
          >
            <div className="stack">
              <PixelInput
                value={llmBaseUrl}
                onChange={setLlmBaseUrl}
                placeholder="API Base URL (e.g. https://api.openai.com/v1)"
              />
              <PixelInput value={llmModel} onChange={setLlmModel} placeholder="Model" />
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveLlm} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>

          <SettingsLine
            title="API key"
            detail="Stored in keyring or environment"
            panel="llmKey"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
          >
            <div className="stack">
              <PixelInput
                value={keyDraft}
                onChange={setKeyDraft}
                placeholder="save to keyring (or env TYPEVOICE_LLM_API_KEY)"
              />
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={setApiKey} tone="accent" disabled={!keyDraft.trim()}>
                  Save
                </PixelButton>
                <PixelButton onClick={clearApiKey} tone="danger">
                  Clear
                </PixelButton>
                <PixelButton onClick={checkApiKey} disabled={llmCheckPending}>
                  {llmCheckPending ? "Checking" : "Check"}
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="Rewrite"
            detail={rewriteEnabled ? "On" : "Off"}
            panel="rewrite"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={<PixelToggle value={rewriteEnabled} onChange={setRewriteEnabled} label="rewrite" />}
          >
            <div className="stack">
              <PixelTextarea
                value={llmPrompt}
                onChange={setLlmPrompt}
                placeholder="LLM prompt..."
                rows={10}
              />
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveRewrite} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="Improvement context"
            detail="Inputs available to rewriting"
            panel="context"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
          >
            <div className="stack">
              <div className="settingsInlineToggle">
                <span>Recent dictated text</span>
                <PixelToggle
                  value={contextIncludeHistory}
                  onChange={setContextIncludeHistory}
                  label="recent dictated text"
                />
              </div>
              <div className="settingsInlineToggle">
                <span>Clipboard text</span>
                <PixelToggle
                  value={contextIncludeClipboard}
                  onChange={setContextIncludeClipboard}
                  label="clipboard text"
                />
              </div>
              <div className="settingsInlineToggle">
                <span>Current app name and title</span>
                <PixelToggle
                  value={contextIncludePrevWindowMeta}
                  onChange={setContextIncludePrevWindowMeta}
                  label="current app name and title"
                />
              </div>
              <div className="settingsInlineToggle">
                <span>Current screen image</span>
                <PixelToggle
                  value={contextIncludePrevWindowScreenshot}
                  onChange={setContextIncludePrevWindowScreenshot}
                  label="current screen image"
                />
              </div>
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveContextConfig} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="Glossary"
            detail="One term per line"
            panel="glossary"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={
              <PixelToggle
                value={rewriteIncludeGlossary}
                onChange={setRewriteIncludeGlossary}
                label="rewrite glossary"
              />
            }
          >
            <div className="stack">
              <div className="muted">
                每行一个词；空行会自动忽略。用于 rewrite 阶段作为上下文词汇或术语约束模型遵循。
              </div>
              <PixelTextarea
                value={rewriteGlossaryDraft}
                onChange={setRewriteGlossaryDraft}
                placeholder={"比如：QPSK\nTypeScript\nOAuth"}
                rows={8}
              />
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveGlossary} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="Export"
            detail={autoPasteEnabled ? "Auto paste on" : "Auto paste off"}
            panel="export"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={
              <PixelToggle
                value={autoPasteEnabled}
                onChange={setAutoPasteEnabled}
                label="auto paste"
              />
            }
          >
            <div className="stack">
              <div className="muted">Use platform APIs to paste automatically.</div>
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveExportConfig} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="Hotkeys"
            detail={hotkeysEnabled ? "On" : "Off"}
            panel="hotkeys"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
            control={<PixelToggle value={hotkeysEnabled} onChange={setHotkeysEnabled} label="hotkeys" />}
          >
            <div className="stack">
              <div className="hotkeyGuide">
                <div><span>Alt</span><span>short press starts or stops recording</span></div>
                <div><span>Enter</span><span>rewrites the transcript window text</span></div>
                <div><span>Shift+Enter</span><span>adds a line break</span></div>
                <div><span>Ctrl+Enter</span><span>inserts the transcript window text</span></div>
              </div>
              <div className="settingsInlineToggle">
                <span>Overlay</span>
                <PixelToggle
                  value={hotkeysShowOverlay}
                  onChange={setHotkeysShowOverlay}
                  label="overlay"
                />
              </div>
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={saveHotkeys} tone="accent">
                  Save
                </PixelButton>
              </div>
            </div>
          </SettingsLine>
        </div>

        <div className="card">
          <SettingsLine
            title="History"
            detail="Stored dictation records"
            panel="history"
            activePanel={activeSettingsPanel}
            onTogglePanel={toggleSettingsPanel}
          >
            <div className="row" style={{ justifyContent: "flex-end" }}>
              <PixelButton onClick={() => setConfirmClear(true)} tone="danger">
                Clear all
              </PixelButton>
            </div>
          </SettingsLine>
        </div>
      </div>

      <PixelDialog
        open={confirmClear}
        title="Clear history"
        onClose={() => setConfirmClear(false)}
        actions={
          <>
            <PixelButton onClick={() => setConfirmClear(false)}>Cancel</PixelButton>
            <PixelButton onClick={clearHistory} tone="danger">
              Clear
            </PixelButton>
          </>
        }
      >
        <div className="stack">
          <div>This will delete all history items.</div>
          <div className="muted">This action cannot be undone.</div>
        </div>
      </PixelDialog>
    </div>
  );
}
