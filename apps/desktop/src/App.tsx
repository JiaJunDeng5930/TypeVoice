import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { Settings } from "./types";
import { PixelTabs, type TabKey } from "./ui/PixelTabs";
import { PixelToastHost, type ToastItem, type ToastTone } from "./ui/PixelToast";
import { MainScreen } from "./screens/MainScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { SettingsScreen } from "./screens/SettingsScreen";

function uid() {
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export default function App() {
  const [tab, setTab] = useState<TabKey>("main");
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [epoch, setEpoch] = useState(0);

  const pushToast = useCallback((message: string, tone: ToastTone = "default") => {
    const id = uid();
    setToasts((prev) => [{ id, message, tone }, ...prev].slice(0, 3));
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const reloadSettings = useCallback(async () => {
    try {
      const s = (await invoke("get_settings")) as Settings;
      setSettings(s);
    } catch {
      setSettings({});
    }
  }, []);

  useEffect(() => {
    reloadSettings();
  }, [reloadSettings]);

  const savePatch = useCallback(
    async (patch: Record<string, unknown>) => {
      const next = (await invoke("update_settings", { patch })) as Settings;
      setSettings(next);
    },
    [],
  );

  const onHistoryChanged = useCallback(() => {
    setEpoch((x) => x + 1);
  }, []);

  const subtitle = useMemo(() => {
    return tab === "main" ? "ONE BUTTON" : tab === "history" ? "ALL RUNS" : "CONFIG";
  }, [tab]);

  return (
    <div className="appBg">
      <div className="layout">
        <div className="topbar">
          <div className="brand">
            <div className="brandTitle">TYPEVOICE</div>
            <div className="brandSub">{subtitle}</div>
          </div>
          <PixelTabs active={tab} onChange={setTab} />
        </div>

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
        </div>
      </div>

      <PixelToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
