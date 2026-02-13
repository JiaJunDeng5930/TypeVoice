type Props = {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  disabled?: boolean;
  readOnly?: boolean;
};

export function PixelInput({
  value,
  onChange,
  placeholder,
  disabled,
  readOnly,
}: Props) {
  return (
    <input
      className="pxInput"
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      placeholder={placeholder}
      disabled={disabled}
      readOnly={readOnly}
      spellCheck={false}
      autoCapitalize="none"
      autoCorrect="off"
    />
  );
}

type TextareaProps = {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  disabled?: boolean;
  rows?: number;
};

export function PixelTextarea({
  value,
  onChange,
  placeholder,
  disabled,
  rows,
}: TextareaProps) {
  return (
    <textarea
      className="pxTextarea"
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      placeholder={placeholder}
      disabled={disabled}
      rows={rows}
      spellCheck={false}
    />
  );
}
