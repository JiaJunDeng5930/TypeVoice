import { useEffect, useMemo, useRef, useState } from "react";
import { defaultTauriGateway } from "./infra/runtimePorts";

type OverlayState = {
  visible: boolean;
  status: string;
  detail?: string | null;
  ts_ms: number;
};

type OverlayAudioLevelEvent = {
  recordingId: string;
  rms: number;
  peak: number;
  tsMs: number;
};

const EQ_BAR_MIX = [
  [0.2, 0.8],
  [0.45, 0.55],
  [0, 1],
  [0.45, 0.55],
  [0.2, 0.8],
] as const;
const RMS_REF = 0.15;
const PEAK_REF = 0.45;
const RMS_ATTACK = 0.35;
const RMS_RELEASE = 0.12;
const PEAK_ATTACK = 0.6;
const PEAK_RELEASE = 0.25;

function clamp01(v: number): number {
  if (!Number.isFinite(v)) return 0;
  return Math.min(1, Math.max(0, v));
}

function levelCurve(v: number): number {
  return Math.pow(clamp01(v), 0.5);
}

function smoothLevel(next: number, prev: number, attack: number, release: number): number {
  const alpha = next >= prev ? attack : release;
  return prev + (next - prev) * alpha;
}

function toneFromStatus(s: string): "default" | "ok" | "danger" {
  const up = String(s || "").toUpperCase();
  if (up.includes("COPY") || up.includes("COPIED") || up.includes("OK")) return "ok";
  if (up.includes("ERR") || up.includes("FAIL") || up.includes("DENIED")) return "danger";
  return "default";
}

function isRecordingStatus(status: string): boolean {
  const up = String(status || "").trim().toUpperCase();
  return up === "REC" || up === "RECORDING" || up.startsWith("REC ");
}

export default function OverlayApp() {
  const [st, setSt] = useState<OverlayState>({
    visible: false,
    status: "IDLE",
    detail: null,
    ts_ms: Date.now(),
  });
  const [rmsLevel, setRmsLevel] = useState(0);
  const [peakLevel, setPeakLevel] = useState(0);
  const stateRef = useRef(st);

  useEffect(() => {
    document.body.classList.add("isOverlay");
    return () => document.body.classList.remove("isOverlay");
  }, []);

  useEffect(() => {
    stateRef.current = st;
  }, [st]);

  useEffect(() => {
    let unlisten: null | (() => void) = null;
    (async () => {
      unlisten = await defaultTauriGateway.listen<OverlayState>("tv_overlay_state", (next) => {
        if (!next) return;
        setSt(next);
        if (!next.visible || !isRecordingStatus(next.status)) {
          setRmsLevel(0);
          setPeakLevel(0);
        }
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

  useEffect(() => {
    let unlisten: null | (() => void) = null;
    (async () => {
      unlisten = await defaultTauriGateway.listen<OverlayAudioLevelEvent>(
        "tv_overlay_audio_level",
        (next) => {
          if (!next) return;
          const current = stateRef.current;
          if (!current.visible || !isRecordingStatus(current.status)) return;
          const rmsRaw = clamp01((next.rms || 0) / RMS_REF);
          const peakRaw = clamp01((next.peak || 0) / PEAK_REF);
          const rmsMapped = levelCurve(rmsRaw);
          const peakMapped = levelCurve(peakRaw);
          setRmsLevel((prev) => smoothLevel(rmsMapped, prev, RMS_ATTACK, RMS_RELEASE));
          setPeakLevel((prev) => smoothLevel(peakMapped, prev, PEAK_ATTACK, PEAK_RELEASE));
        },
      );
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
  const statusUpper = useMemo(() => String(st.status || "").toUpperCase(), [st.status]);
  const isRec = st.visible && isRecordingStatus(st.status);
  const bars = useMemo(() => {
    if (!isRec) return [0, 0, 0, 0, 0];
    return EQ_BAR_MIX.map(([r, p]) => clamp01(r * rmsLevel + p * peakLevel));
  }, [isRec, peakLevel, rmsLevel]);

  return (
    <div className={`overlayRoot ${st.visible ? "isVisible" : ""} tone-${tone} ${isRec ? "isRec" : ""}`}>
      <div className="overlayTop">
        <div className="overlayEq" aria-hidden={!isRec}>
          {bars.map((v, idx) => (
            <span
              key={idx}
              className="overlayEqBar"
              style={{ height: `${10 + Math.round(v * 22)}px`, opacity: isRec ? 1 : 0.2 }}
            />
          ))}
        </div>
        <div className="overlayBadge">{statusUpper}</div>
      </div>
      {st.detail ? <div className="overlayDetail">{st.detail}</div> : null}
    </div>
  );
}
