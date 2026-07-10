export type PixelSelectOption = { value: string; label: string };

type Props = {
  value: string;
  onChange: (next: string) => void;
  options: PixelSelectOption[];
  placeholder?: string;
  ariaLabel?: string;
  disabled?: boolean;
};

export function PixelSelect({
  value,
  onChange,
  options,
  placeholder,
  ariaLabel,
  disabled,
}: Props) {
  return (
    <select
      className="pxSelectBtn"
      value={value}
      onChange={(event) => onChange(event.currentTarget.value)}
      disabled={disabled}
      aria-label={ariaLabel || placeholder}
    >
      {placeholder && !value ? (
        <option value="" disabled>
          {placeholder}
        </option>
      ) : null}
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}
