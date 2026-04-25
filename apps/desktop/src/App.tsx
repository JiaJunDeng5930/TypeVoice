import { useCallback, useEffect, useMemo, useState } from "react";
import { defaultTauriGateway } from "./infra/runtimePorts";
import type { Settings } from "./types";
import { PixelTabs, type TabKey } from "./ui/PixelTabs";
import { PixelToastHost, type ToastItem, type ToastTone } from "./ui/PixelToast";
import { MainScreen } from "./screens/MainScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { SettingsScreen } from "./screens/SettingsScreen";

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
      const msg = typeof err === "string" ? err : String(err);
      setSettingsError(msg);
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

  const subtitle = useMemo(() => {
    return tab === "main" ? "READY" : tab === "history" ? "ALL RUNS" : "SAVED";
  }, [tab]);

  return (
    <div className="appBg">
      <div className="layout appShell">
        <aside className="sideRail">
          <div className="brand">
            <div className="brandTitle">TYPEVOICE</div>
          </div>
          <PixelTabs active={tab} onChange={setTab} />
          <div className="brandSub">{subtitle}</div>
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
            {settingsError ? <div className="muted">SETTINGS ERROR: {settingsError}</div> : null}
          </div>
        </main>
      </div>

      <PixelToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
