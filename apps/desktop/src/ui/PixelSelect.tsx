import { useEffect, useId, useMemo, useRef, useState } from "react";

export type PixelSelectOption = { value: string; label: string };

type Props = {
  value: string;
  onChange: (next: string) => void;
  options: PixelSelectOption[];
  placeholder?: string;
  disabled?: boolean;
};

export function PixelSelect({
  value,
  onChange,
  options,
  placeholder,
  disabled,
}: Props) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const id = useId();

  const selectedLabel = useMemo(() => {
    return options.find((o) => o.value === value)?.label || "";
  }, [options, value]);

  useEffect(() => {
    function onDocDown(e: MouseEvent) {
      const t = e.target as Node | null;
      if (!t) return;
      if (btnRef.current?.contains(t)) return;
      if (listRef.current?.contains(t)) return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocDown);
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  return (
    <div className="pxSelectRoot">
      <button
        type="button"
        className="pxSelectBtn"
        ref={btnRef}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-list`}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={selectedLabel || placeholder || ""}
      >
        <span className="pxSelectValue">
          {selectedLabel || placeholder || "-"}
        </span>
        <span className="pxSelectArrow" aria-hidden="true">
          v
        </span>
      </button>

      {open ? (
        <div className="pxSelectList" role="listbox" id={`${id}-list`} ref={listRef}>
          {options.map((o) => (
            <button
              key={o.value}
              type="button"
              className={`pxSelectItem ${o.value === value ? "isSelected" : ""}`}
              role="option"
              aria-selected={o.value === value}
              onClick={() => {
                onChange(o.value);
                setOpen(false);
              }}
            >
              {o.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
