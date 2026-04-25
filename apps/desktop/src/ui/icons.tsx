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
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path
        d="M12 3.75a3.2 3.2 0 0 0-3.2 3.2v4.3a3.2 3.2 0 0 0 6.4 0v-4.3A3.2 3.2 0 0 0 12 3.75Z"
        fill="none"
        stroke={c}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.6"
      />
      <path
        d="M6.4 10.6v.75a5.6 5.6 0 0 0 11.2 0v-.75M12 17v3.25M9.1 20.25h5.8"
        fill="none"
        stroke={c}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.6"
      />
    </svg>
  );
}

export function IconStop({ size = 64, tone = "ink" }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <rect x="7" y="7" width="10" height="10" rx="2.2" fill={c} />
      <circle cx="12" cy="12" r="8.25" fill="none" stroke={c} strokeWidth="1.6" />
    </svg>
  );
}

export function IconTranscribing({ size = 64, tone = "accent" }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      className="pxSpin"
    >
      <path
        d="M12 3.8a8.2 8.2 0 1 1-7.1 4.1"
        fill="none"
        stroke={c}
        strokeLinecap="round"
        strokeWidth="1.8"
      />
      <path
        d="M4.2 4.2v3.9h3.9"
        fill="none"
        stroke={c}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.8"
      />
    </svg>
  );
}

