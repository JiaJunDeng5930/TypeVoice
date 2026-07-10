import { useEffect, useRef, type ReactNode } from "react";

type Props = {
  open: boolean;
  title: string;
  children: ReactNode;
  onClose: () => void;
  actions: ReactNode;
};

export function PixelDialog({ open, title, children, onClose, actions }: Props) {
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    return () => {
      if (dialog.open) dialog.close();
    };
  }, [open]);

  if (!open) return null;
  return (
    <dialog
      ref={dialogRef}
      className="pxDialog"
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="pxDialogTop">
        <div className="pxDialogTitle">{title}</div>
        <button type="button" className="pxDialogX" onClick={onClose} aria-label="Close dialog">
          ×
        </button>
      </div>
      <div className="pxDialogBody">{children}</div>
      <div className="pxDialogActions">{actions}</div>
    </dialog>
  );
}

