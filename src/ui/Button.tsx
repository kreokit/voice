import { forwardRef } from "react";
import type { LucideIcon } from "lucide-react";
import { Tooltip } from "./Tooltip";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: "sm" | "md";
  icon?: LucideIcon;
  /** Icon after the label (e.g. a chevron on menu buttons). */
  trailingIcon?: LucideIcon;
}

export const ICON_STROKE = 1.75;

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon: Icon, trailingIcon: Trailing, className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      className={["ui-button", `ui-button-${variant}`, `ui-button-${size}`, className]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    >
      {Icon && <Icon size={size === "sm" ? 14 : 16} strokeWidth={ICON_STROKE} aria-hidden />}
      {children}
      {Trailing && <Trailing size={14} strokeWidth={ICON_STROKE} aria-hidden />}
    </button>
  );
});

export interface IconButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: LucideIcon;
  /** Accessible name, also shown as the tooltip. */
  label: string;
  shortcut?: string;
  size?: "sm" | "md";
  variant?: "ghost" | "secondary" | "primary";
  pressed?: boolean;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { icon: Icon, label, shortcut, size = "md", variant = "ghost", pressed, className, ...rest },
  ref,
) {
  return (
    <Tooltip label={label} shortcut={shortcut}>
      <button
        ref={ref}
        type="button"
        aria-label={label}
        aria-pressed={pressed}
        className={["ui-icon-button", `ui-button-${variant}`, `ui-icon-button-${size}`, className]
          .filter(Boolean)
          .join(" ")}
        {...rest}
      >
        <Icon size={size === "sm" ? 14 : 18} strokeWidth={ICON_STROKE} aria-hidden />
      </button>
    </Tooltip>
  );
});

export function Kbd({ children }: { children: React.ReactNode }) {
  return <kbd className="ui-kbd">{children}</kbd>;
}
