import { useEffect, useMemo, useState } from "react";
import { defaultTauriGateway } from "./infra/runtimePorts";

type OverlayState = {
  visible: boolean;
  status: string;
  detail?: string | null;
  ts_ms: number;
};

function toneFromStatus(s: string): "default" | "ok" | "danger" {
  const up = String(s || "").toUpperCase();
  if (up.includes("COPY") || up.includes("COPIED") || up.includes("OK")) return "ok";
  if (up.includes("ERR") || up.includes("FAIL") || up.includes("DENIED")) return "danger";
  return "default";
}

export default function OverlayApp() {
  const [st, setSt] = useState<OverlayState>({
    visible: false,
    status: "IDLE",
    detail: null,
    ts_ms: Date.now(),
  });

  useEffect(() => {
    document.body.classList.add("isOverlay");
    return () => document.body.classList.remove("isOverlay");
  }, []);

  useEffect(() => {
    let unlisten: null | (() => void) = null;
    (async () => {
      unlisten = await defaultTauriGateway.listen<OverlayState>("tv_overlay_state", (next) => {
        if (!next) return;
        setSt(next);
      });
    })();
    return () => {
      try {
        unlisten?.();
      } catch {
        // ignore
      }
    };
  }, []);

  const tone = useMemo(() => toneFromStatus(st.status), [st.status]);

  return (
    <div className={`overlayRoot ${st.visible ? "isVisible" : ""} tone-${tone}`}>
      <div className="overlayBadge">{String(st.status || "").toUpperCase()}</div>
      {st.detail ? <div className="overlayDetail">{st.detail}</div> : null}
    </div>
  );
}
