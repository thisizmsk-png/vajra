import type { KeyboardEvent, ReactNode } from "react";
import { cn } from "../cn";

/** A 28 px row. The parent supplies the container role: `listbox` gives rows `option`, a grid gives `row`. */
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
  return (
    <div
      role={role}
      aria-selected={selected ?? false}
      tabIndex={selected ? 0 : -1}
      className={cn("v-row", className)}
      onClick={onSelect}
      onKeyDown={key}
    >
      <span>{children}</span>
      {meta != null && <span className="v-row__meta">{meta}</span>}
    </div>
  );
}
