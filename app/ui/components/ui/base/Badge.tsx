import type { ReactNode } from "react";
import { cn } from "../cn";

/** Outline label next to text. The text is always present; color never carries the meaning alone. */
export function Badge({ status, className, children }: { status?: "success" | "warning" | "danger" | "info"; className?: string; children: ReactNode }) {
  return <span className={cn("v-badge", status && `v-badge--${status}`, className)}>{children}</span>;
}
