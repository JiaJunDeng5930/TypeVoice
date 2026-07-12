import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import { defaultTauriGateway } from "../infra/runtimePorts";
import type {
  ApiCheckResult,
  ApiKeyStatus,
  AudioCaptureDevice,
  Settings,
} from "../types";
import { PixelButton } from "../ui/PixelButton";
import { PixelDialog } from "../ui/PixelDialog";
import { PixelInput, PixelTextarea } from "../ui/PixelInput";
import { PixelSelect, type PixelSelectOption } from "../ui/PixelSelect";
import { PixelToggle } from "../ui/PixelToggle";

type Props = {
  settings: Settings | null;
  settingsError?: string | null;
  onRetrySettings: () => void;
  savePatch: (patch: Record<string, unknown>) => Promise<void>;
  pushToast: (msg: string, tone?: "default" | "ok" | "danger") => void;
  onHistoryCleared: () => void;
};

const REASONING: PixelSelectOption[] = [
  { value: "default", label: "Default (omit)" },
  { value: "none", label: "None" },
  { value: "minimal", label: "Minimal" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "Extra high" },
];

const ASR_PROVIDERS: PixelSelectOption[] = [
  { value: "doubao", label: "Doubao streaming" },
  { value: "remote", label: "Remote (cloud)" },
];

const RECORD_INPUT_STRATEGIES: PixelSelectOption[] = [
  { value: "follow_default", label: "Follow system default" },
  { value: "fixed_device", label: "Use a specific device" },
  { value: "auto_select", label: "Select an available device" },
];

const RECORD_DEFAULT_ROLES: PixelSelectOption[] = [
  { value: "communications", label: "Communications (eCommunications)" },
  { value: "console", label: "Console (eConsole)" },
];

const PRIMARY_HOTKEYS: PixelSelectOption[] = [
  { value: "Alt", label: "Alt" },
  { value: "Ctrl", label: "Ctrl" },
  { value: "Shift", label: "Shift" },
  { value: "F1", label: "F1" },
  { value: "F2", label: "F2" },
  { value: "F3", label: "F3" },
  { value: "F4", label: "F4" },
  { value: "F5", label: "F5" },
  { value: "F6", label: "F6" },
  { value: "F7", label: "F7" },
  { value: "F8", label: "F8" },
  { value: "F9", label: "F9" },
  { value: "F10", label: "F10" },
  { value: "F11", label: "F11" },
  { value: "F12", label: "F12" },
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

type EffectiveSettingsValues = {
  llm_base_url?: string | null;
  llm_model?: string | null;
};

type ToastActionMessages = {
  success: string;
  failure: string;
};

type SettingsLineProps = {
  title: string;
  detail?: string;
  panel: SettingsPanelId;
  expandedPanels: SettingsPanelId[];
  onTogglePanel: (panel: SettingsPanelId) => void;
  control?: ReactNode;
  children?: ReactNode;
};

type SliderFieldProps = {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  suffix?: string;
  onChange: (value: number) => void;
};

function SliderField({
  label,
  value,
  min,
  max,
  step,
  suffix = "",
  onChange,
}: SliderFieldProps) {
  return (
    <label className="sliderField">
      <span>{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
      />
      <strong>{formatSliderValue(value, suffix)}</strong>
    </label>
  );
}

function formatSliderValue(value: number, suffix: string): string {
  const display = Number.isInteger(value) ? String(value) : value.toFixed(2);
  return `${display}${suffix}`;
}

function SettingsLine({
  title,
  detail,
  panel,
  expandedPanels,
  onTogglePanel,
  control,
  children,
}: SettingsLineProps) {
  const expanded = expandedPanels.includes(panel);
  const panelId = `settings-panel-${panel}`;
  return (
    <div className={`settingsLineBlock ${expanded ? "isExpanded" : ""}`}>
      <div className="settingsLine">
        <h3 className="settingsLineHeading">
          <button
            type="button"
            className="settingsLineSummary"
            aria-label={`${title} settings`}
            aria-expanded={expanded}
            aria-controls={panelId}
            onClick={() => onTogglePanel(panel)}
          >
            <span className="settingsLineText">
              <span className="settingsLineTitle">{title}</span>
              {detail ? <span className="settingsLineDetail">{detail}</span> : null}
            </span>
            <span className="settingsChevron" aria-hidden="true" />
          </button>
        </h3>
        {control ? <div className="settingsLineControl">{control}</div> : null}
      </div>
      {expanded && children ? (
        <div id={panelId} className="settingsLinePanel">
          {children}
        </div>
      ) : null}
    </div>
  );
}

function sensitiveSettingDisplay(status: ApiKeyStatus | null): string {
  if (!status?.configured) return "";
  const source = status.source.trim();
  return source ? `Configured via ${source}` : "Configured";
}

function clampNumber(value: number | null | undefined, fallback: number, min: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, value));
}

export function SettingsScreen({
  settings,
  settingsError,
  onRetrySettings,
  savePatch,
  pushToast,
  onHistoryCleared,
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
  const [audioCaptureDevicesError, setAudioCaptureDevicesError] = useState<string | null>(null);

  const [hotkeysEnabled, setHotkeysEnabled] = useState(true);
  const [hotkeyPrimary, setHotkeyPrimary] = useState("Alt");
  const [hotkeysShowOverlay, setHotkeysShowOverlay] = useState(true);
  const [overlayBackgroundOpacity, setOverlayBackgroundOpacity] = useState(0.78);
  const [overlayFontSizePx, setOverlayFontSizePx] = useState(32);
  const [overlayWidthPx, setOverlayWidthPx] = useState(960);
  const [overlayHeightPx, setOverlayHeightPx] = useState(160);
  const [contextIncludeHistory, setContextIncludeHistory] = useState(true);
  const [contextIncludeClipboard, setContextIncludeClipboard] = useState(true);
  const [contextIncludePrevWindowMeta, setContextIncludePrevWindowMeta] = useState(true);
  const [contextIncludePrevWindowScreenshot, setContextIncludePrevWindowScreenshot] =
    useState(true);
  const [rewriteIncludeGlossary, setRewriteIncludeGlossary] = useState(true);

  const [keyDraft, setKeyDraft] = useState("");
  const [llmKeyStatus, setLlmKeyStatus] = useState<ApiKeyStatus | null>(null);
  const [remoteAsrKeyStatus, setRemoteAsrKeyStatus] = useState<ApiKeyStatus | null>(null);
  const [doubaoCredentialsStatus, setDoubaoCredentialsStatus] = useState<ApiKeyStatus | null>(null);

  const [confirmClear, setConfirmClear] = useState(false);
  const [llmCheckPending, setLlmCheckPending] = useState(false);
  const [remoteAsrCheckPending, setRemoteAsrCheckPending] = useState(false);
  const [doubaoCheckPending, setDoubaoCheckPending] = useState(false);
  const [expandedSettingsPanels, setExpandedSettingsPanels] = useState<SettingsPanelId[]>([]);

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
    setHotkeyPrimary(normalizePrimaryHotkey(settings.hotkey_primary));
    setHotkeysShowOverlay(settings.hotkeys_show_overlay);
    setOverlayBackgroundOpacity(
      clampNumber(settings.overlay_background_opacity, 0.78, 0.35, 0.95),
    );
    setOverlayFontSizePx(clampNumber(settings.overlay_font_size_px, 32, 18, 56));
    setOverlayWidthPx(clampNumber(settings.overlay_width_px, 960, 360, 1600));
    setOverlayHeightPx(clampNumber(settings.overlay_height_px, 160, 72, 360));

    setContextIncludeHistory(settings.context_include_history ?? true);
    setContextIncludeClipboard(settings.context_include_clipboard ?? true);
    setContextIncludePrevWindowMeta(settings.context_include_prev_window_meta ?? true);
    setContextIncludePrevWindowScreenshot(
      settings.context_include_prev_window_screenshot ?? true,
    );
  }, [settings, pushToast]);

  useEffect(() => {
    if (!settings) return;
    (async () => {
      await refreshSensitiveSettingStatuses();
      try {
        const effective = (await defaultTauriGateway.invoke(
          "effective_settings_values",
        )) as EffectiveSettingsValues;
        if (!settings.llm_base_url?.trim() && effective.llm_base_url?.trim()) {
          setLlmBaseUrl(effective.llm_base_url.trim());
        }
        if (!settings.llm_model?.trim() && effective.llm_model?.trim()) {
          setLlmModel(effective.llm_model.trim());
        }
      } catch {
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings]);

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
      const rows = (await defaultTauriGateway.invoke(
        "list_audio_capture_devices",
      )) as AudioCaptureDevice[];
      setAudioCaptureDevices(rows);
      setAudioCaptureDevicesError(null);
      if (!recordFixedEndpointId.trim()) return;
      const found = rows.find((v) => v.endpoint_id === recordFixedEndpointId);
      if (!found) return;
      setRecordFixedFriendlyName(found.friendly_name);
    } catch {
      setAudioCaptureDevicesError("Recording devices could not be loaded.");
    }
  }

  async function refreshSensitiveSettingStatuses() {
    try {
      const [llmStatus, remoteStatus, doubaoStatus] = await Promise.all([
        defaultTauriGateway.invoke("llm_api_key_status") as Promise<ApiKeyStatus>,
        defaultTauriGateway.invoke("remote_asr_api_key_status") as Promise<ApiKeyStatus>,
        defaultTauriGateway.invoke("doubao_asr_credentials_status") as Promise<ApiKeyStatus>,
      ]);
      setLlmKeyStatus(llmStatus);
      setRemoteAsrKeyStatus(remoteStatus);
      setDoubaoCredentialsStatus(doubaoStatus);
    } catch {
    }
  }

  async function runToastAction(
    action: () => Promise<void>,
    messages: ToastActionMessages,
  ): Promise<boolean> {
    try {
      await action();
      pushToast(messages.success, "ok");
      return true;
    } catch {
      pushToast(messages.failure, "danger");
      return false;
    }
  }

  async function persistSettingsPatch(
    patch: Record<string, unknown>,
    successMessage = "Saved",
  ): Promise<boolean> {
    return runToastAction(
      async () => {
        await savePatch(patch);
      },
      { success: successMessage, failure: "Save failed" },
    );
  }

  async function saveAsr() {
    const provider = asrProvider === "remote" ? "remote" : "doubao";
    const concurrencyNum = Number(remoteAsrConcurrency);
    if (provider === "remote" && !remoteAsrUrl.trim()) {
      pushToast("Remote ASR URL is required", "danger");
      return;
    }
    if (!Number.isFinite(concurrencyNum)) {
      pushToast("Remote ASR concurrency must be a number", "danger");
      return;
    }
    const normalizedConcurrency = Math.max(1, Math.min(16, Math.round(concurrencyNum)));
    const saved = await persistSettingsPatch({
      asr_provider: provider,
      remote_asr_url: remoteAsrUrl.trim() ? remoteAsrUrl.trim() : null,
      remote_asr_model: remoteAsrModel.trim() ? remoteAsrModel.trim() : null,
      remote_asr_concurrency: normalizedConcurrency,
    });
    if (saved) {
      setRemoteAsrConcurrency(String(normalizedConcurrency));
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
      pushToast("Select a fixed recording device", "danger");
      return;
    }
    const selected = audioCaptureDevices.find(
      (v) => v.endpoint_id === recordFixedEndpointId.trim(),
    );
    const saved = await persistSettingsPatch({
      record_input_strategy: strategy,
      record_follow_default_role: role,
      record_fixed_endpoint_id:
        strategy === "fixed_device" ? recordFixedEndpointId.trim() : null,
      record_fixed_friendly_name:
        strategy === "fixed_device"
          ? (selected?.friendly_name || recordFixedFriendlyName || "").trim() || null
          : null,
    });
    if (saved) {
      if (selected) {
        setRecordFixedFriendlyName(selected.friendly_name);
      }
      await refreshAudioCaptureDevices();
    }
  }

  async function savePreprocessConfig() {
    const thresholdDb = Number(asrPreprocessThresholdDb);
    const trimStartMs = Number(asrPreprocessStartMs);
    const trimEndMs = Number(asrPreprocessEndMs);
    if (!Number.isFinite(thresholdDb) || !Number.isFinite(trimStartMs) || !Number.isFinite(trimEndMs)) {
      pushToast("Silence trim values must be numbers", "danger");
      return;
    }
    if (thresholdDb > 0) {
      pushToast("Silence threshold cannot be above 0 dB", "danger");
      return;
    }
    if (trimStartMs < 0 || trimEndMs < 0) {
      pushToast("Silence trim duration cannot be negative", "danger");
      return;
    }
    await persistSettingsPatch({
      asr_preprocess_silence_trim_enabled: asrPreprocessTrimEnabled,
      asr_preprocess_silence_threshold_db: thresholdDb,
      asr_preprocess_silence_start_ms: Number.isInteger(trimStartMs)
        ? trimStartMs
        : Math.round(trimStartMs),
      asr_preprocess_silence_end_ms: Number.isInteger(trimEndMs)
        ? trimEndMs
        : Math.round(trimEndMs),
    });
  }

  async function saveLlm() {
    await persistSettingsPatch({
      llm_base_url: llmBaseUrl.trim() ? llmBaseUrl.trim() : null,
      llm_model: llmModel.trim() ? llmModel.trim() : null,
      llm_reasoning_effort: reasoning === "default" ? null : reasoning,
    });
  }

  async function saveRewrite() {
    if (rewriteEnabled && !llmPrompt.trim()) {
      pushToast("A rewrite prompt is required", "danger");
      return;
    }
    await persistSettingsPatch({
      rewrite_enabled: rewriteEnabled,
      llm_prompt: llmPrompt,
      rewrite_include_glossary: rewriteIncludeGlossary,
    });
  }

  async function saveGlossary() {
    const items = rewriteGlossaryDraft
      .split("\n")
      .map((x) => x.trim())
      .filter((x) => x.length > 0);
    await persistSettingsPatch(
      {
        rewrite_glossary: items,
        rewrite_include_glossary: rewriteIncludeGlossary,
      },
      "Glossary saved",
    );
  }

  async function saveContextConfig() {
    await persistSettingsPatch({
      context_include_history: contextIncludeHistory,
      context_include_clipboard: contextIncludeClipboard,
      context_include_prev_window_meta: contextIncludePrevWindowMeta,
      context_include_prev_window_screenshot: contextIncludePrevWindowScreenshot,
    });
  }

  async function saveExportConfig() {
    await persistSettingsPatch({
      auto_paste_enabled: autoPasteEnabled,
    });
  }

  async function saveHotkeys() {
    await persistSettingsPatch({
      hotkeys_enabled: hotkeysEnabled,
      hotkey_primary: normalizePrimaryHotkey(hotkeyPrimary),
      hotkeys_show_overlay: hotkeysShowOverlay,
      overlay_background_opacity: overlayBackgroundOpacity,
      overlay_font_size_px: Math.round(overlayFontSizePx),
      overlay_width_px: Math.round(overlayWidthPx),
      overlay_height_px: Math.round(overlayHeightPx),
    });
  }

  async function setApiKey() {
    const k = keyDraft.trim();
    if (!k) return;
    try {
      await defaultTauriGateway.invoke("set_llm_api_key", { apiKey: k });
      setKeyDraft("");
      await refreshSensitiveSettingStatuses();
      pushToast("API key saved", "ok");
    } catch {
      pushToast("API key could not be saved", "danger");
    }
  }

  async function clearApiKey() {
    try {
      await defaultTauriGateway.invoke("clear_llm_api_key");
      setLlmKeyStatus(null);
      await refreshSensitiveSettingStatuses();
      pushToast("API key cleared", "ok");
    } catch {
      pushToast("API key could not be cleared", "danger");
    }
  }

  async function checkApiKey() {
    if (llmCheckPending) return;
    setLlmCheckPending(true);
    try {
      const result = (await defaultTauriGateway.invoke("check_llm_api_key", {
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
      await defaultTauriGateway.invoke("set_remote_asr_api_key", { apiKey: k });
      setRemoteAsrKeyDraft("");
      await refreshSensitiveSettingStatuses();
      pushToast("Remote ASR key saved", "ok");
    } catch {
      pushToast("Remote ASR key could not be saved", "danger");
    }
  }

  async function clearRemoteAsrApiKey() {
    try {
      await defaultTauriGateway.invoke("clear_remote_asr_api_key");
      setRemoteAsrKeyStatus(null);
      await refreshSensitiveSettingStatuses();
      pushToast("Remote ASR key cleared", "ok");
    } catch {
      pushToast("Remote ASR key could not be cleared", "danger");
    }
  }

  async function checkRemoteAsrApiKey() {
    if (remoteAsrCheckPending) return;
    setRemoteAsrCheckPending(true);
    try {
      const result = (await defaultTauriGateway.invoke("check_remote_asr_api_key", {
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
      await defaultTauriGateway.invoke("set_doubao_asr_credentials", { appKey, accessKey });
      setDoubaoAppKeyDraft("");
      setDoubaoAccessKeyDraft("");
      await refreshSensitiveSettingStatuses();
      pushToast("Doubao credentials saved", "ok");
    } catch {
      pushToast("Doubao credentials could not be saved", "danger");
    }
  }

  async function clearDoubaoAsrCredentials() {
    try {
      await defaultTauriGateway.invoke("clear_doubao_asr_credentials");
      setDoubaoCredentialsStatus(null);
      await refreshSensitiveSettingStatuses();
      pushToast("Doubao credentials cleared", "ok");
    } catch {
      pushToast("Doubao credentials could not be cleared", "danger");
    }
  }

  async function checkDoubaoAsrCredentials() {
    if (doubaoCheckPending) return;
    setDoubaoCheckPending(true);
    try {
      const result = (await defaultTauriGateway.invoke("check_doubao_asr_credentials")) as ApiCheckResult;
      pushToast(result.message, result.ok ? "ok" : "danger");
    } catch {
      pushToast("Doubao ASR API check failed. Try again after checking the settings.", "danger");
    } finally {
      setDoubaoCheckPending(false);
    }
  }

  async function clearHistory() {
    try {
      await defaultTauriGateway.invoke("history_clear");
      pushToast("History cleared", "ok");
      onHistoryCleared();
    } catch {
      pushToast("History could not be cleared", "danger");
    } finally {
      setConfirmClear(false);
    }
  }

  const asrStatusText = useMemo(() => {
    if (asrProvider === "doubao") {
      return "Doubao streaming";
    }
    return `Remote ${remoteAsrUrl.trim() || "https://api.server/transcribe"}`;
  }, [asrProvider, remoteAsrUrl]);

  const llmKeyDisplay = sensitiveSettingDisplay(llmKeyStatus);
  const remoteAsrKeyDisplay = sensitiveSettingDisplay(remoteAsrKeyStatus);
  const doubaoCredentialsDisplay = sensitiveSettingDisplay(doubaoCredentialsStatus);

  function toggleSettingsPanel(panel: SettingsPanelId) {
    setExpandedSettingsPanels((current) =>
      current.includes(panel)
        ? current.filter((value) => value !== panel)
        : [...current, panel],
    );
  }

  function expandSettingsPanel(panel: SettingsPanelId) {
    setExpandedSettingsPanels((current) =>
      current.includes(panel) ? current : [...current, panel],
    );
  }

  if (settings === null) {
    return (
      <div className="pageSurface settingsSurface">
        <header className="pageHeader settingsHeader">
          <h1 className="pageTitle">Settings</h1>
        </header>
        <div className="card">
          {settingsError ? (
            <div className="stack">
              <h2 className="settingsGroupTitle">Settings unavailable</h2>
              <div className="muted">{settingsError}</div>
              <div className="row" style={{ justifyContent: "flex-end" }}>
                <PixelButton onClick={onRetrySettings} tone="accent">
                  Retry
                </PixelButton>
              </div>
            </div>
          ) : (
            <div className="stack">
              <h2 className="settingsGroupTitle">Loading settings</h2>
              <div className="muted">Reading your saved configuration…</div>
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="pageSurface settingsSurface">
      <header className="pageHeader settingsHeader">
        <h1 className="pageTitle">Settings</h1>
        <div className="settingsHeaderNote">Changes save by section</div>
      </header>
      <div className="settingsGrid">
        <div className="settingsColumn">
          <div className="card">
            <h2 className="settingsGroupTitle">Voice &amp; input</h2>
            <SettingsLine
              title="Speech recognition"
              detail={asrStatusText}
              panel="asr"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelSelect
                  value={asrProvider}
                  onChange={(value) => {
                    setAsrProvider(value);
                    expandSettingsPanel("asr");
                  }}
                  options={ASR_PROVIDERS}
                  ariaLabel="Speech recognition provider"
                />
              }
            >
              <div className="stack">
                {asrProvider === "doubao" ? (
                  <>
                    <PixelInput
                      value={doubaoAppKeyDraft}
                      onChange={setDoubaoAppKeyDraft}
                      label="Doubao app key"
                      placeholder="Enter a new app key"
                      type="password"
                      autoComplete="new-password"
                    />
                    <PixelInput
                      value={doubaoAccessKeyDraft}
                      onChange={setDoubaoAccessKeyDraft}
                      label="Doubao access key"
                      placeholder="Enter a new access key"
                      type="password"
                      autoComplete="new-password"
                    />
                    <div className="muted">
                      {doubaoCredentialsDisplay || "Doubao credentials are not configured"}
                    </div>
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
                      label="Remote ASR URL"
                      placeholder="https://api.server/transcribe"
                      type="url"
                      inputMode="url"
                      autoComplete="url"
                    />
                    <PixelInput
                      value={remoteAsrModel}
                      onChange={setRemoteAsrModel}
                      label="Remote ASR model"
                      placeholder="Optional model name"
                      type="text"
                      autoComplete="off"
                    />
                    <PixelInput
                      value={remoteAsrConcurrency}
                      onChange={setRemoteAsrConcurrency}
                      label="Slicing concurrency"
                      placeholder="1–16"
                      type="number"
                      inputMode="numeric"
                      autoComplete="off"
                    />
                    <PixelInput
                      value={remoteAsrKeyDraft}
                      onChange={setRemoteAsrKeyDraft}
                      label="Remote ASR API key"
                      placeholder="Enter a new API key"
                      type="password"
                      autoComplete="new-password"
                    />
                    <div className="muted">
                      {remoteAsrKeyDisplay || "Remote ASR key is not configured"}
                    </div>
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
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelSelect
                  value={recordInputStrategy}
                  onChange={(value) => {
                    setRecordInputStrategy(value);
                    expandSettingsPanel("recording");
                  }}
                  options={RECORD_INPUT_STRATEGIES}
                  ariaLabel="Recording input strategy"
                />
              }
            >
              <div className="stack">
                {recordInputStrategy === "follow_default" ? (
                  <PixelSelect
                    value={recordFollowDefaultRole}
                    onChange={setRecordFollowDefaultRole}
                    options={RECORD_DEFAULT_ROLES}
                    ariaLabel="System default recording role"
                  />
                ) : null}
                {recordInputStrategy === "fixed_device" ? (
                  <>
                    <PixelSelect
                      value={recordFixedEndpointId}
                      onChange={setRecordFixedEndpointId}
                      options={captureDeviceOptions}
                      placeholder="Select a fixed capture endpoint"
                      ariaLabel="Fixed recording device"
                    />
                    {recordFixedFriendlyName ? (
                      <div className="muted">Fixed device: {recordFixedFriendlyName}</div>
                    ) : null}
                  </>
                ) : null}
                {audioCaptureDevicesError ? (
                  <div className="muted">{audioCaptureDevicesError}</div>
                ) : audioCaptureDevices.length === 0 ? (
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
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelToggle
                  value={asrPreprocessTrimEnabled}
                  onChange={(value) => {
                    setAsrPreprocessTrimEnabled(value);
                    expandSettingsPanel("preprocess");
                  }}
                  label="Silence trim"
                />
              }
            >
              <div className="stack">
                <PixelInput
                  value={asrPreprocessThresholdDb}
                  onChange={setAsrPreprocessThresholdDb}
                  label="Silence threshold (dB)"
                  placeholder="-50"
                  type="number"
                  inputMode="decimal"
                  autoComplete="off"
                />
                <PixelInput
                  value={asrPreprocessStartMs}
                  onChange={setAsrPreprocessStartMs}
                  label="Leading silence (ms)"
                  placeholder="300"
                  type="number"
                  inputMode="numeric"
                  autoComplete="off"
                />
                <PixelInput
                  value={asrPreprocessEndMs}
                  onChange={setAsrPreprocessEndMs}
                  label="Trailing silence (ms)"
                  placeholder="300"
                  type="number"
                  inputMode="numeric"
                  autoComplete="off"
                />
                <div className="row" style={{ justifyContent: "flex-end" }}>
                  <PixelButton onClick={savePreprocessConfig} tone="accent">
                    Save
                  </PixelButton>
                </div>
              </div>
            </SettingsLine>
          </div>

          <div className="card">
            <h2 className="settingsGroupTitle">Writing</h2>
            <SettingsLine
              title="Rewrite"
              detail={rewriteEnabled ? "On" : "Off"}
              panel="rewrite"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelToggle
                  value={rewriteEnabled}
                  onChange={(value) => {
                    setRewriteEnabled(value);
                    expandSettingsPanel("rewrite");
                  }}
                  label="Rewrite"
                />
              }
            >
              <div className="stack">
                <PixelTextarea
                  value={llmPrompt}
                  onChange={setLlmPrompt}
                  label="Rewrite prompt"
                  placeholder="Describe how dictated text should be rewritten"
                  rows={10}
                />
                <div className="row" style={{ justifyContent: "flex-end" }}>
                  <PixelButton onClick={saveRewrite} tone="accent">
                    Save
                  </PixelButton>
                </div>
              </div>
            </SettingsLine>
            <SettingsLine
              title="Glossary"
              detail="One term per line"
              panel="glossary"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelToggle
                  value={rewriteIncludeGlossary}
                  onChange={(value) => {
                    setRewriteIncludeGlossary(value);
                    expandSettingsPanel("glossary");
                  }}
                  label="Rewrite glossary"
                />
              }
            >
              <div className="stack">
                <div className="muted">
                  Add one term per line. Empty lines are ignored, and saved terms guide rewriting.
                </div>
                <PixelTextarea
                  value={rewriteGlossaryDraft}
                  onChange={setRewriteGlossaryDraft}
                  label="Glossary terms"
                  placeholder={"For example:\nQPSK\nTypeScript\nOAuth"}
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
            <h2 className="settingsGroupTitle">Shortcuts &amp; overlay</h2>
            <SettingsLine
              title="Hotkeys"
              detail={hotkeysEnabled ? "On" : "Off"}
              panel="hotkeys"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelToggle
                  value={hotkeysEnabled}
                  onChange={(value) => {
                    setHotkeysEnabled(value);
                    expandSettingsPanel("hotkeys");
                  }}
                  label="Hotkeys"
                />
              }
            >
              <div className="stack">
                <div className="hotkeyGuide">
                  <div><span>{hotkeyPrimary}</span><span>Short press to start or stop recording.</span></div>
                </div>
                <div className="stack">
                  <div className="muted">Primary key</div>
                  <PixelSelect
                    value={hotkeyPrimary}
                    onChange={setHotkeyPrimary}
                    options={PRIMARY_HOTKEYS}
                    ariaLabel="Primary hotkey"
                  />
                </div>
                <div className="settingsInlineToggle">
                  <span>Current-session subtitles</span>
                  <PixelToggle
                    value={hotkeysShowOverlay}
                    onChange={setHotkeysShowOverlay}
                    label="Current-session subtitles"
                  />
                </div>
                <SliderField
                  label="Background depth"
                  min={0.35}
                  max={0.95}
                  step={0.01}
                  value={overlayBackgroundOpacity}
                  onChange={setOverlayBackgroundOpacity}
                />
                <SliderField
                  label="Font size"
                  min={18}
                  max={56}
                  step={1}
                  value={overlayFontSizePx}
                  suffix="px"
                  onChange={setOverlayFontSizePx}
                />
                <SliderField
                  label="Width"
                  min={360}
                  max={1600}
                  step={10}
                  value={overlayWidthPx}
                  suffix="px"
                  onChange={setOverlayWidthPx}
                />
                <SliderField
                  label="Height"
                  min={72}
                  max={360}
                  step={4}
                  value={overlayHeightPx}
                  suffix="px"
                  onChange={setOverlayHeightPx}
                />
                <div className="row" style={{ justifyContent: "flex-end" }}>
                  <PixelButton onClick={saveHotkeys} tone="accent">
                    Save
                  </PixelButton>
                </div>
              </div>
            </SettingsLine>
          </div>
        </div>

        <div className="settingsColumn">
          <div className="card">
            <h2 className="settingsGroupTitle">Intelligence</h2>
            <SettingsLine
              title="Language model"
              detail={llmModel.trim() || "Model settings"}
              panel="llm"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelSelect
                  value={reasoning}
                  onChange={(value) => {
                    setReasoning(value);
                    expandSettingsPanel("llm");
                  }}
                  options={REASONING}
                  ariaLabel="Reasoning effort"
                />
              }
            >
              <div className="stack">
                <PixelInput
                  value={llmBaseUrl}
                  onChange={setLlmBaseUrl}
                  label="API base URL"
                  placeholder="https://api.openai.com/v1"
                  type="url"
                  inputMode="url"
                  autoComplete="url"
                />
                <PixelInput
                  value={llmModel}
                  onChange={setLlmModel}
                  label="Model"
                  placeholder="Model name"
                  type="text"
                  autoComplete="off"
                />
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
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
            >
              <div className="stack">
                <PixelInput
                  value={keyDraft}
                  onChange={setKeyDraft}
                  label="LLM API key"
                  placeholder="Enter a new API key"
                  type="password"
                  autoComplete="new-password"
                />
                <div className="muted">{llmKeyDisplay || "LLM API key is not configured"}</div>
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
            <SettingsLine
              title="Improvement context"
              detail="Inputs available to rewriting"
              panel="context"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
            >
              <div className="stack">
                <div className="settingsInlineToggle">
                  <span>Recent dictated text</span>
                  <PixelToggle
                    value={contextIncludeHistory}
                    onChange={setContextIncludeHistory}
                    label="Recent dictated text"
                  />
                </div>
                <div className="settingsInlineToggle">
                  <span>Clipboard text</span>
                  <PixelToggle
                    value={contextIncludeClipboard}
                    onChange={setContextIncludeClipboard}
                    label="Clipboard text"
                  />
                </div>
                <div className="settingsInlineToggle">
                  <span>Current app name and title</span>
                  <PixelToggle
                    value={contextIncludePrevWindowMeta}
                    onChange={setContextIncludePrevWindowMeta}
                    label="Current app name and title"
                  />
                </div>
                <div className="settingsInlineToggle">
                  <span>Current screen image</span>
                  <PixelToggle
                    value={contextIncludePrevWindowScreenshot}
                    onChange={setContextIncludePrevWindowScreenshot}
                    label="Current screen image"
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
            <h2 className="settingsGroupTitle">Delivery &amp; data</h2>
            <SettingsLine
              title="Export"
              detail={autoPasteEnabled ? "Auto paste on" : "Auto paste off"}
              panel="export"
              expandedPanels={expandedSettingsPanels}
              onTogglePanel={toggleSettingsPanel}
              control={
                <PixelToggle
                  value={autoPasteEnabled}
                  onChange={(value) => {
                    setAutoPasteEnabled(value);
                    expandSettingsPanel("export");
                  }}
                  label="Auto paste"
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
            <SettingsLine
              title="History"
              detail="Stored dictation records"
              panel="history"
              expandedPanels={expandedSettingsPanels}
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

function normalizePrimaryHotkey(value: string | null | undefined): string {
  const raw = (value || "").trim();
  const found = PRIMARY_HOTKEYS.find((item) => item.value.toLowerCase() === raw.toLowerCase());
  return found?.value || "Alt";
}
