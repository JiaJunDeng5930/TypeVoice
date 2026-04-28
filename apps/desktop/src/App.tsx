import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { defaultTauriGateway } from "./infra/runtimePorts";
import type { Settings } from "./types";
import { PixelTabs, type TabKey } from "./ui/PixelTabs";
import { PixelToastHost, type ToastItem, type ToastTone } from "./ui/PixelToast";
import { MainScreen } from "./screens/MainScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { userMessageFromError } from "./domain/diagnostic";

let toastSeq = 0;

function uid() {
  toastSeq += 1;
  return `toast-${toastSeq}`;
}

export default function App() {
  const [tab, setTab] = useState<TabKey>("main");
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [epoch, setEpoch] = useState(0);

  const pushToast = useCallback((message: string, tone: ToastTone = "default") => {
    const id = uid();
    setToasts((prev) => [{ id, message, tone }, ...prev].slice(0, 3));
    if (tone === "danger") {
      void defaultTauriGateway
        .invoke("ui_log_event", {
          req: {
            kind: "toast",
            code: "E_UI_TOAST_DANGER",
            message,
            tone,
            tab,
            screen: tab,
            tsMs: Date.now(),
            extra: { toastId: id },
          },
        })
        .catch(() => {
          // ignore ui logging failure
        });
    }
  }, [tab]);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const reloadSettings = useCallback(async () => {
    try {
      const s = (await defaultTauriGateway.invoke("get_settings")) as Settings;
      setSettings(s);
      setSettingsError(null);
    } catch (err) {
      setSettings(null);
      setSettingsError(userMessageFromError(err, "Settings need attention"));
    }
  }, []);

  useEffect(() => {
    reloadSettings();
  }, [reloadSettings]);

  const savePatch = useCallback(
    async (patch: Record<string, unknown>) => {
      const next = (await defaultTauriGateway.invoke("update_settings", { patch })) as Settings;
      setSettings(next);
      setSettingsError(null);
    },
    [],
  );

  const onHistoryChanged = useCallback(() => {
    setEpoch((x) => x + 1);
  }, []);

  return (
    <div className="appBg">
      <header
        className="windowTitlebar"
        data-tauri-drag-region
        onDoubleClick={() => void getCurrentWindow().toggleMaximize()}
      >
        <div className="windowTitle" data-tauri-drag-region>TypeVoice</div>
        <div className="windowControls" onDoubleClick={(event) => event.stopPropagation()}>
          <button
            type="button"
            className="windowControl windowControlMinimize"
            aria-label="Minimize"
            title="Minimize"
            onClick={() => void getCurrentWindow().minimize()}
          >
            <span />
          </button>
          <button
            type="button"
            className="windowControl windowControlMaximize"
            aria-label="Maximize"
            title="Maximize"
            onClick={() => void getCurrentWindow().toggleMaximize()}
          >
            <span />
          </button>
          <button
            type="button"
            className="windowControl windowControlClose"
            aria-label="Close"
            title="Close"
            onClick={() => void getCurrentWindow().close()}
          >
            <span />
          </button>
        </div>
      </header>
      <div className="layout appShell">
        <aside className="sideRail">
          <div className="brand">
            <div className="brandTitle">TYPEVOICE</div>
          </div>
          <PixelTabs active={tab} onChange={setTab} />
          <div />
        </aside>

        <main className="contentStage">
          <div style={{ display: tab === "main" ? "block" : "none" }}>
            <MainScreen
              settings={settings}
              pushToast={pushToast}
              onHistoryChanged={onHistoryChanged}
            />
          </div>
          <div style={{ display: tab === "history" ? "block" : "none" }}>
            <HistoryScreen epoch={epoch} pushToast={pushToast} />
          </div>
          <div style={{ display: tab === "settings" ? "block" : "none" }}>
            <SettingsScreen
              settings={settings}
              savePatch={savePatch}
              pushToast={pushToast}
              onHistoryCleared={onHistoryChanged}
            />
            {settingsError ? <div className="muted">{settingsError}</div> : null}
          </div>
        </main>
      </div>

      <PixelToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
