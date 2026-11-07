// Small presentational helpers shared by the three directions. Each direction styles them through its own CSS.
import type { ReactNode } from "react";

export { StatusDot } from "../components/ui/base/StatusDot";

/** A keyboard hint. Rendered as <kbd> so screen readers announce it as keys. */
export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="m-kbd">{children}</kbd>;
}

/** Added/removed line counts, with + and − glyphs so the meaning never depends on color. */
export function DiffCount({ added, removed }: { added?: number; removed?: number }) {
  if (added === undefined && removed === undefined) return null;
  return (
    <span className="m-diffcount" aria-label={`${added ?? 0} lines added, ${removed ?? 0} removed`}>
      <span className="m-add">+{added ?? 0}</span> <span className="m-del">−{removed ?? 0}</span>
    </span>
  );
}

/** The Vajra mark: a simple two-pronged bolt, drawn in currentColor. */
export function Mark({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <path d="M9.6 1 3.5 9h3.7L6.2 15l6.3-8.2H8.8L9.6 1Z" fill="currentColor" />
    </svg>
  );
}

export function Chevron() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
      <path d="M2.5 3.8 5 6.3l2.5-2.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}

export type Theme = "dark" | "light";
