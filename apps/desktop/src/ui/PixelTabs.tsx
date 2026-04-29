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
    label: "Main",
    icon: (active) => <IconNavMic size={34} tone={active ? "accent" : "muted"} filled={active} />,
  },
  {
    key: "history",
    label: "History",
    icon: (active) => <IconBookOpen size={34} tone={active ? "accent" : "muted"} filled={active} />,
  },
  {
    key: "settings",
    label: "Settings",
    icon: (active) => <IconGear size={34} tone={active ? "accent" : "muted"} filled={active} />,
  },
];

export function PixelTabs({ active, onChange }: Props) {
  return (
    <div className="pxTabs" role="tablist" aria-label="pages">
      {tabs.map((tab) => {
        const selected = active === tab.key;
        return (
          <button
            key={tab.key}
            type="button"
            role="tab"
            aria-label={tab.label}
            title={tab.label}
            aria-selected={selected}
            className={`pxTab ${selected ? "isActive" : ""}`}
            onClick={() => onChange(tab.key)}
          >
            {tab.icon(selected)}
          </button>
        );
      })}
    </div>
  );
}

export type { TabKey };

