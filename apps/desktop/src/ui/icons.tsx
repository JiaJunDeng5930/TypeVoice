type IconProps = { size?: number; tone?: "ink" | "accent" | "muted"; filled?: boolean };

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

export function IconNavMic({ size = 28, tone = "ink", filled = false }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M12 3.6a3.25 3.25 0 0 0-3.25 3.25v4.3a3.25 3.25 0 0 0 6.5 0v-4.3A3.25 3.25 0 0 0 12 3.6Z"
        fill={filled ? c : "none"}
        stroke={c}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.55"
      />
      <path
        d="M6.8 10.4v.85a5.2 5.2 0 0 0 10.4 0v-.85M12 16.45v3.2M9.3 19.65h5.4"
        fill="none"
        stroke={c}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.55"
      />
    </svg>
  );
}

export function IconBookOpen({ size = 28, tone = "ink", filled = false }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M4.2 5.45c2.9-.35 5.2.15 7.8 1.8 2.6-1.65 4.9-2.15 7.8-1.8v12.9c-2.9-.35-5.2.15-7.8 1.8-2.6-1.65-4.9-2.15-7.8-1.8Z"
        fill={filled ? c : "none"}
        stroke={c}
        strokeLinejoin="round"
        strokeWidth="1.45"
      />
      <path
        d="M12 7.25v12.9"
        fill="none"
        stroke={filled ? "var(--surface)" : c}
        strokeLinecap="round"
        strokeWidth="1.45"
      />
    </svg>
  );
}

export function IconGear({ size = 28, tone = "ink", filled = false }: IconProps) {
  const c = colorFromTone(tone);
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M10.55 3.7h2.9l.55 2.05c.45.16.88.34 1.28.57l1.86-1.05 2.05 2.05-1.05 1.86c.23.4.41.83.57 1.28l2.05.55v2.9l-2.05.55c-.16.45-.34.88-.57 1.28l1.05 1.86-2.05 2.05-1.86-1.05c-.4.23-.83.41-1.28.57l-.55 2.05h-2.9L10 19.13a7.3 7.3 0 0 1-1.28-.57l-1.86 1.05-2.05-2.05 1.05-1.86a7.3 7.3 0 0 1-.57-1.28l-2.05-.55v-2.9l2.05-.55c.16-.45.34-.88.57-1.28L4.81 7.28l2.05-2.05 1.86 1.05c.4-.23.83-.41 1.28-.57Z"
        fill={filled ? c : "none"}
        stroke={c}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.4"
      />
      <circle
        cx="12"
        cy="12"
        r="2.8"
        fill={filled ? "var(--surface)" : "none"}
        stroke={c}
        strokeWidth="1.4"
      />
    </svg>
  );
}

