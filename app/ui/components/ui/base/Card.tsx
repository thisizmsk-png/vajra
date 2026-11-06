import type { ReactNode } from "react";
import { cn } from "../cn";

/** `interactive` renders a real button, so it is focusable and announced as one. */
export function Card({ interactive, onClick, className, children }: { interactive?: boolean; onClick?: () => void; className?: string; children?: ReactNode }) {
  if (interactive) {
    return (
      <button type="button" className={cn("v-card", "v-card--interactive", className)} onClick={onClick}>
        {children}
      </button>
    );
  }
  return <div className={cn("v-card", className)}>{children}</div>;
}
