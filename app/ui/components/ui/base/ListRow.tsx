import { useId } from "react";
import type { ReactNode } from "react";
import { cn } from "../cn";

type Common = { meta?: ReactNode; className?: string; children: ReactNode };
type Props =
  | (Common & { role?: "option"; selected?: boolean; onSelect?: () => void })
  // A grid row is not focusable here and has no selection: the grid that owns it supplies focus and keys, so
  // `selected` and `onSelect` are type errors (a mouse-only row would fail WCAG 2.1.1).
  | (Common & { role: "row"; selected?: never; onSelect?: never });

/**
 * A 28 px row. The parent decides the role: `option` inside a `ListBox` (default; the ListBox owns focus and the
 * keys, `selected` is exposed), or `row` inside a grid or table (content wrapped in `gridcell`s, no `aria-selected`).
 */
export function ListRow({ role = "option", selected, meta, onSelect, className, children }: Props) {
  const id = useId();
  const isRow = role === "row";
  const cell = isRow ? "gridcell" : undefined;
  return (
    <div id={id} role={role} aria-selected={isRow ? undefined : selected} className={cn("v-row", className)} onClick={onSelect}>
      <span role={cell}>{children}</span>
      {meta != null && (
        <span role={cell} className="v-row__meta">
          {meta}
        </span>
      )}
    </div>
  );
}
