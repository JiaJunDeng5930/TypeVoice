import type { HTMLInputTypeAttribute, InputHTMLAttributes } from "react";

type Props = {
  value: string;
  onChange: (v: string) => void;
  label?: string;
  placeholder?: string;
  type?: HTMLInputTypeAttribute;
  inputMode?: InputHTMLAttributes<HTMLInputElement>["inputMode"];
  autoComplete?: string;
  disabled?: boolean;
  readOnly?: boolean;
};

export function PixelInput({
  value,
  onChange,
  label,
  placeholder,
  type,
  inputMode,
  autoComplete,
  disabled,
  readOnly,
}: Props) {
  const input = (
    <input
      className="pxInput"
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      placeholder={placeholder}
      aria-label={label || placeholder}
      type={type}
      inputMode={inputMode}
      autoComplete={autoComplete}
      disabled={disabled}
      readOnly={readOnly}
      spellCheck={false}
      autoCapitalize="none"
      autoCorrect="off"
    />
  );

  if (!label) return input;

  return (
    <label className="settingsField">
      <span className="muted">{label}</span>
      {input}
    </label>
  );
}

type TextareaProps = {
  value: string;
  onChange: (v: string) => void;
  label?: string;
  placeholder?: string;
  disabled?: boolean;
  rows?: number;
};

export function PixelTextarea({
  value,
  onChange,
  label,
  placeholder,
  disabled,
  rows,
}: TextareaProps) {
  const textarea = (
    <textarea
      className="pxTextarea"
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      placeholder={placeholder}
      aria-label={label || placeholder}
      disabled={disabled}
      rows={rows}
      spellCheck={false}
    />
  );

  if (!label) return textarea;

  return (
    <label className="settingsField">
      <span className="muted">{label}</span>
      {textarea}
    </label>
  );
}
