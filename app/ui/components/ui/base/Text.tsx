import type { ElementType, ReactNode } from "react";
import { cn } from "../cn";

export type TextVariant = "headline-lg" | "headline-md" | "body" | "body-sm" | "label" | "mono";

/** All type goes through here. `as` picks the semantic element. */
export function Text({
  variant = "body",
  as: Tag = "p",
  muted,
  numeric,
  className,
  children,
}: {
  variant?: TextVariant;
  as?: ElementType;
  muted?: boolean;
  /** Tabular numerals, for counts, timestamps and exit codes. */
  numeric?: boolean;
  className?: string;
  children?: ReactNode;
}) {
  return <Tag className={cn("v-text", `v-text--${variant}`, muted && "v-text--muted", numeric && "v-num", className)}>{children}</Tag>;
}
