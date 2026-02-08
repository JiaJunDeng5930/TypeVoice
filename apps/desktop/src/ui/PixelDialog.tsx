import type { ReactNode } from "react";

type Props = {
  open: boolean;
  title: string;
  children: ReactNode;
  onClose: () => void;
  actions: ReactNode;
};

export function PixelDialog({ open, title, children, onClose, actions }: Props) {
  if (!open) return null;
  return (
    <div className="pxDialogBackdrop" role="dialog" aria-modal="true">
      <div className="pxDialog">
        <div className="pxDialogTop">
          <div className="pxDialogTitle">{title}</div>
          <button type="button" className="pxDialogX" onClick={onClose} aria-label="close">
            X
          </button>
        </div>
        <div className="pxDialogBody">{children}</div>
        <div className="pxDialogActions">{actions}</div>
      </div>
    </div>
  );
}

