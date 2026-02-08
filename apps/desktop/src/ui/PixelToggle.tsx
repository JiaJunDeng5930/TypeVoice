type Props = {
  value: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label?: string;
};

export function PixelToggle({ value, onChange, disabled, label }: Props) {
  return (
    <button
      type="button"
      className={`pxToggle ${value ? "isOn" : "isOff"}`}
      role="switch"
      aria-checked={value}
      aria-label={label || "toggle"}
      disabled={disabled}
      onClick={() => onChange(!value)}
    >
      <span className="pxToggleKnob" />
    </button>
  );
}

