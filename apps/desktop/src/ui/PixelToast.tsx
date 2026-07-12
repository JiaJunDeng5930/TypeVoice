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
    const timers = toasts
      .filter((toast) => toast.tone !== "danger")
      .map((toast) => window.setTimeout(() => onDismiss(toast.id), 1800));
    return () => timers.forEach((x) => window.clearTimeout(x));
  }, [toasts, onDismiss]);

  return (
    <div className="pxToastHost">
      {toasts.slice(0, 2).map((t) => (
        <div
          key={t.id}
          className={`pxToast ${t.tone === "ok" ? "isOk" : t.tone === "danger" ? "isDanger" : ""}`}
          role={t.tone === "danger" ? "alert" : "status"}
        >
          <span className="pxToastMessage">{t.message}</span>
          <button
            type="button"
            className="pxToastDismiss"
            onClick={() => onDismiss(t.id)}
            aria-label="Close notification"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}

