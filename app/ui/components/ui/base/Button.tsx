import type { ButtonHTMLAttributes, MouseEvent, ReactNode } from "react";
import { cn } from "../cn";

type Base = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "disabled" | "children"> & {
  "data-autofocus"?: boolean;
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  /** Uses aria-disabled so the button stays focusable and announced. */
  disabled?: boolean;
  /** While busy the click is ignored; pass the busy wording as children (for example "Stopping"). */
  busy?: boolean;
};

// An icon-only button must carry an accessible name.
type Props = Base & ({ children: ReactNode } | { children?: undefined; "aria-label": string });

export function Button({ variant = "secondary", size = "md", disabled, busy, onClick, className, children, ...rest }: Props) {
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
      {children}
    </button>
  );
}
