export async function copyText(text: string): Promise<void> {
  const t = text ?? "";
  if (!t.trim()) return;

  // Prefer async clipboard API
  try {
    await navigator.clipboard.writeText(t);
    return;
  } catch {
    // fallthrough
  }

  // Fallback for environments where clipboard API is blocked.
  const el = document.createElement("textarea");
  el.value = t;
  el.style.position = "fixed";
  el.style.left = "-9999px";
  el.style.top = "0";
  document.body.appendChild(el);
  el.focus();
  el.select();
  try {
    document.execCommand("copy");
  } finally {
    document.body.removeChild(el);
  }
}

