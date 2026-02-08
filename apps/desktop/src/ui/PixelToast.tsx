import { useEffect } from "react";

export type ToastTone = "default" | "ok" | "danger";

export type ToastItem = {
  id: string;
  message: string;
  tone: ToastTone;
};

type Props = {
  toasts: ToastItem[];
  onDismiss: (id: string) => void;
};

export function PixelToastHost({ toasts, onDismiss }: Props) {
  useEffect(() => {
    if (!toasts.length) return;
    const timers = toasts.map((t) =>
      window.setTimeout(() => onDismiss(t.id), 1800),
    );
    return () => timers.forEach((x) => window.clearTimeout(x));
  }, [toasts, onDismiss]);

  return (
    <div className="pxToastHost" aria-live="polite" aria-relevant="additions">
      {toasts.slice(0, 2).map((t) => (
        <div
          key={t.id}
          className={`pxToast ${t.tone === "ok" ? "isOk" : t.tone === "danger" ? "isDanger" : ""}`}
          onClick={() => onDismiss(t.id)}
          role="status"
        >
          {t.message}
        </div>
      ))}
    </div>
  );
}

