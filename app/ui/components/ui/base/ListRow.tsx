import type { KeyboardEvent, ReactNode } from "react";
import { cn } from "../cn";

/** A 28 px `option` row; put it inside a `listbox`. `selected` is exposed only when the list is selectable. */
export function ListRow({ selected, meta, onSelect, className, children }: { selected?: boolean; meta?: ReactNode; onSelect?: () => void; className?: string; children: ReactNode }) {
  const key = (e: KeyboardEvent) => {
    if (onSelect && (e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      onSelect();
    }
  };
  return (
    <div role="option" aria-selected={selected} tabIndex={0} className={cn("v-row", className)} onClick={onSelect} onKeyDown={key}>
      <span>{children}</span>
      {meta != null && <span className="v-row__meta">{meta}</span>}
    </div>
  );
}
