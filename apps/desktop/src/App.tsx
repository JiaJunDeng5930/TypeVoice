import { useCallback, useEffect, useRef, useState } from "react";
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
  const tabRef = useRef<TabKey>(tab);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [isMaximized, setIsMaximized] = useState(false);
  const [epoch, setEpoch] = useState(0);

  const pushToast = useCallback((message: string, tone: ToastTone = "default") => {
    const id = uid();
    setToasts((prev) => {
      if (prev.some((toast) => toast.message === message && toast.tone === tone)) return prev;
      return [{ id, message, tone }, ...prev].slice(0, 3);
    });
    if (tone === "danger") {
      void defaultTauriGateway
        .invoke("ui_log_event", {
          req: {
            kind: "toast",
            code: "E_UI_TOAST_DANGER",
            message,
            tone,
            tab: tabRef.current,
            screen: tabRef.current,
            tsMs: Date.now(),
            extra: { toastId: id },
          },
        })
        .catch(() => {
          // ignore ui logging failure
        });
    }
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const reloadSettings = useCallback(async () => {
    setSettingsError(null);
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

  useEffect(() => {
    tabRef.current = tab;
  }, [tab]);

  useEffect(() => {
    let appWindow: ReturnType<typeof getCurrentWindow>;
    try {
      appWindow = getCurrentWindow();
    } catch {
      return;
    }
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const refreshMaximized = () => {
      void appWindow
        .isMaximized()
        .then((value) => {
          if (!disposed) setIsMaximized(value);
        })
        .catch(() => {
          // Window state only affects the control label.
        });
    };

    refreshMaximized();
    void appWindow
      .onResized(refreshMaximized)
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch(() => {
        // Keep the default label if resize events are unavailable.
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

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

  const runWindowCommand = useCallback(
    (command: () => Promise<void>, failureMessage: string) => {
      void command().catch(() => {
        pushToast(failureMessage, "danger");
      });
    },
    [pushToast],
  );

  return (
    <div className="appBg">
      <header
        className="windowTitlebar"
        data-tauri-drag-region
        onDoubleClick={() => {
          runWindowCommand(
            () => getCurrentWindow().toggleMaximize(),
            "Window resize failed",
          );
        }}
      >
        <div className="windowTitle" data-tauri-drag-region>
          <span className="windowTitleDot" aria-hidden="true" />
          TypeVoice
        </div>
        <div className="windowControls" onDoubleClick={(event) => event.stopPropagation()}>
          <button
            type="button"
            className="windowControl windowControlMinimize"
            aria-label="Minimize"
            title="Minimize"
            onClick={() => {
              runWindowCommand(
                () => getCurrentWindow().minimize(),
                "Window minimize failed",
              );
            }}
          >
            <span />
          </button>
          <button
            type="button"
            className="windowControl windowControlMaximize"
            aria-label={isMaximized ? "Restore" : "Maximize"}
            title={isMaximized ? "Restore" : "Maximize"}
            onClick={() => {
              runWindowCommand(
                () => getCurrentWindow().toggleMaximize(),
                "Window resize failed",
              );
            }}
          >
            <span />
          </button>
          <button
            type="button"
            className="windowControl windowControlClose"
            aria-label="Close"
            title="Close"
            onClick={() => {
              runWindowCommand(
                () => getCurrentWindow().close(),
                "Window close failed",
              );
            }}
          >
            <span />
          </button>
        </div>
      </header>
      <div className="appShell">
        <div className="appNav">
          <PixelTabs active={tab} onChange={setTab} />
        </div>

        <main className="contentStage">
          <div className="screenSlot" hidden={tab !== "main"}>
            <MainScreen
              settings={settings}
              pushToast={pushToast}
              onHistoryChanged={onHistoryChanged}
            />
          </div>
          <div className="screenSlot" hidden={tab !== "history"}>
            <HistoryScreen epoch={epoch} pushToast={pushToast} />
          </div>
          <div className="screenSlot" hidden={tab !== "settings"}>
            <SettingsScreen
              settings={settings}
              settingsError={settingsError}
              savePatch={savePatch}
              pushToast={pushToast}
              onHistoryCleared={onHistoryChanged}
              onRetrySettings={reloadSettings}
            />
          </div>
        </main>
      </div>

      <PixelToastHost toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
