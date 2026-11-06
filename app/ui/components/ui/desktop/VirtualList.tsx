import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

/**
 * Renders only the rows in view (plus `overscan`) of a fixed-row-height list, so a 100,000-line scrollback costs
 * the same as a screenful. Height and row height are props (not measured), which keeps it deterministic.
 */
export function VirtualList({
  count,
  rowHeight,
  height,
  overscan = 8,
  followEnd,
  renderRow,
  className,
  ...aria
}: {
  count: number;
  rowHeight: number;
  height: number;
  overscan?: number;
  /** Stay pinned to the last row while the user is at the bottom (a live log). */
  followEnd?: boolean;
  renderRow: (index: number) => ReactNode;
  className?: string;
  role?: string;
  tabIndex?: number;
  "aria-label"?: string;
  "aria-live"?: "off" | "polite";
  "aria-readonly"?: boolean;
  onKeyDown?: (e: React.KeyboardEvent) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const total = count * rowHeight;
  const atEnd = useRef(true);
  const first = Math.max(0, Math.floor(top / rowHeight) - overscan);
  const last = Math.min(count, Math.ceil((top + height) / rowHeight) + overscan);

  useEffect(() => {
    const el = ref.current;
    if (followEnd && atEnd.current && el) el.scrollTop = Math.max(0, total - height);
  }, [followEnd, total, height]);

  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    rows.push(
      <div key={i} className="v-vrow" style={{ top: i * rowHeight, height: rowHeight }} data-index={i}>
        {renderRow(i)}
      </div>,
    );
  }
  return (
    <div
      ref={ref}
      className={className}
      style={{ height, overflow: "auto", position: "relative" }}
      onScroll={(e) => {
        const el = e.currentTarget;
        atEnd.current = el.scrollTop + height >= total - rowHeight;
        setTop(el.scrollTop);
      }}
      {...aria}
    >
      <div style={{ height: total, position: "relative" }}>{rows}</div>
    </div>
  );
}
