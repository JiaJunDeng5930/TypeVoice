type IconProps = { size?: number; tone?: "ink" | "accent" | "muted" };

function colorFromTone(tone: IconProps["tone"]) {
  if (tone === "accent") return "var(--accent)";
  if (tone === "muted") return "var(--muted)";
  return "var(--ink)";
}

export function IconStart({ size = 64, tone = "ink" }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      <rect x="0" y="0" width="16" height="16" fill="none" />
      <rect x="3" y="3" width="10" height="10" fill="none" stroke={c} strokeWidth="2" />
      <rect x="6" y="6" width="4" height="4" fill={c} />
    </svg>
  );
}

export function IconStop({ size = 64, tone = "ink" }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      <rect x="3" y="3" width="10" height="10" fill={c} />
      <rect x="1" y="1" width="14" height="14" fill="none" stroke={c} strokeWidth="2" />
    </svg>
  );
}

export function IconTranscribing({ size = 64, tone = "accent" }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      shapeRendering="crispEdges"
      aria-hidden="true"
      className="pxSpin"
    >
      <rect x="0" y="0" width="16" height="16" fill="none" />
      <rect x="2" y="2" width="12" height="12" fill="none" stroke={c} strokeWidth="2" />
      <rect x="7" y="4" width="2" height="2" fill={c} />
      <rect x="10" y="7" width="2" height="2" fill={c} />
      <rect x="7" y="10" width="2" height="2" fill={c} />
      <rect x="4" y="7" width="2" height="2" fill={c} />
    </svg>
  );
}

