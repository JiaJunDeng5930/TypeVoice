import type { ReactNode } from "react";

type Props = {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  tone?: "default" | "accent" | "danger";
  className?: string;
  title?: string;
  type?: "button" | "submit";
};

export function PixelButton({
  children,
  onClick,
  disabled,
  tone = "default",
  className,
  title,
  type = "button",
}: Props) {
  const toneClass =
    tone === "accent" ? "pxBtnAccent" : tone === "danger" ? "pxBtnDanger" : "";
  return (
    <button
      type={type}
      className={`pxBtn ${toneClass} ${className || ""}`}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {children}
    </button>
  );
}

