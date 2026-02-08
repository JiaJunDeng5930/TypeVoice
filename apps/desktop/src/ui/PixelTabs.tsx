type TabKey = "main" | "history" | "settings";

type Props = {
  active: TabKey;
  onChange: (t: TabKey) => void;
};

export function PixelTabs({ active, onChange }: Props) {
  return (
    <div className="pxTabs" role="tablist" aria-label="pages">
      <button
        type="button"
        role="tab"
        aria-selected={active === "main"}
        className={`pxTab ${active === "main" ? "isActive" : ""}`}
        onClick={() => onChange("main")}
      >
        MAIN
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={active === "history"}
        className={`pxTab ${active === "history" ? "isActive" : ""}`}
        onClick={() => onChange("history")}
      >
        HISTORY
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={active === "settings"}
        className={`pxTab ${active === "settings" ? "isActive" : ""}`}
        onClick={() => onChange("settings")}
      >
        SETTINGS
      </button>
    </div>
  );
}

export type { TabKey };

