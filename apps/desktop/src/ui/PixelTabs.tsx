import type { ReactNode } from "react";
import { IconBookOpen, IconGear, IconNavMic } from "./icons";

type TabKey = "main" | "history" | "settings";

type Props = {
  active: TabKey;
  onChange: (t: TabKey) => void;
};

const tabs: Array<{
  key: TabKey;
  label: string;
  icon: (active: boolean) => ReactNode;
}> = [
  {
    key: "main",
    label: "Record",
    icon: (active) => <IconNavMic size={17} tone={active ? "accent" : "muted"} filled={active} />,
  },
  {
    key: "history",
    label: "History",
    icon: (active) => <IconBookOpen size={17} tone={active ? "accent" : "muted"} filled={active} />,
  },
  {
    key: "settings",
    label: "Settings",
    icon: (active) => <IconGear size={17} tone={active ? "accent" : "muted"} filled={active} />,
  },
];

export function PixelTabs({ active, onChange }: Props) {
  return (
    <nav className="pxTabs" aria-label="Primary navigation">
      {tabs.map((tab) => {
        const selected = active === tab.key;
        return (
          <button
            key={tab.key}
            type="button"
            aria-label={tab.label}
            title={tab.label}
            aria-current={selected ? "page" : undefined}
            className={`pxTab ${selected ? "isActive" : ""}`}
            onClick={() => onChange(tab.key)}
          >
            <span className="pxTabIcon">{tab.icon(selected)}</span>
            <span className="pxTabLabel">{tab.label}</span>
          </button>
        );
      })}
    </nav>
  );
}

export type { TabKey };
