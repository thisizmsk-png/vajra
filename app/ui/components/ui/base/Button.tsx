import type { ButtonHTMLAttributes, MouseEvent, ReactNode } from "react";
import { cn } from "../cn";

type Base = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "disabled" | "children"> & {
  "data-autofocus"?: boolean;
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  /** Uses aria-disabled so the button stays focusable and announced. */
  disabled?: boolean;
  /** The label changes while busy (for example "Stopping"); the click is ignored. */
  busy?: boolean;
  busyLabel?: string;
};

// An icon-only button must carry an accessible name.
type Props = Base & ({ children: ReactNode } | { children?: undefined; "aria-label": string });

export function Button({ variant = "secondary", size = "md", disabled, busy, busyLabel, onClick, className, children, ...rest }: Props) {
  const off = disabled || busy;
  const click = (e: MouseEvent<HTMLButtonElement>) => {
    if (off) e.preventDefault();
    else onClick?.(e);
  };
  return (
    <button
      type="button"
      {...rest}
      aria-disabled={off || undefined}
      aria-busy={busy || undefined}
      className={cn("v-button", `v-button--${variant}`, size === "sm" && "v-button--sm", className)}
      onClick={click}
    >
      {busy && busyLabel ? busyLabel : children}
    </button>
  );
}
