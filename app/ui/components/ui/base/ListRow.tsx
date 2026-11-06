import type { KeyboardEvent, ReactNode } from "react";
import { cn } from "../cn";

/**
 * A 28 px row. The parent decides the role: `option` inside a `listbox` (default; `selected` is exposed), or `row`
 * inside a grid or table (the content is wrapped in a `gridcell`, and no `aria-selected` is set).
 */
export function ListRow({
  role = "option",
  selected,
  meta,
  onSelect,
  className,
  children,
}: {
  role?: "option" | "row";
  selected?: boolean;
  meta?: ReactNode;
  onSelect?: () => void;
  className?: string;
  children: ReactNode;
}) {
  const key = (e: KeyboardEvent) => {
    if (onSelect && (e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      onSelect();
    }
  };
  const isRow = role === "row";
  const label = <span role={isRow ? "gridcell" : undefined}>{children}</span>;
  const extra = meta != null && (
    <span role={isRow ? "gridcell" : undefined} className="v-row__meta">
      {meta}
    </span>
  );
  return (
    <div role={role} aria-selected={isRow ? undefined : selected} tabIndex={0} className={cn("v-row", className)} onClick={onSelect} onKeyDown={key}>
      {label}
      {extra}
    </div>
  );
}
