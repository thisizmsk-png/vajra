import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { HTMLAttributes, ReactNode } from "react";

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
  onRange,
  origin,
  className,
  ...aria
}: Omit<HTMLAttributes<HTMLDivElement>, "children" | "style" | "onScroll"> & {
  count: number;
  rowHeight: number;
  height: number;
  overscan?: number;
  /** Stay pinned to the last row while the user is at the bottom (a live log). */
  followEnd?: boolean;
  renderRow: (index: number) => ReactNode;
  /** The rows in view (first inclusive, end exclusive, no overscan) and whether the view is at the bottom; called when any changes. */
  onRange?: (start: number, end: number, atBottom: boolean) => void;
  /** An id for row 0 that rows are anchored to (a stream's first seq). When it moves, a reader who is not at the bottom keeps the same lines in view. */
  origin?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const total = count * rowHeight;
  const atEnd = useRef(true);
  const first = Math.max(0, Math.floor(top / rowHeight) - overscan);
  const last = Math.min(count, Math.ceil((top + height) / rowHeight) + overscan);

  useEffect(() => {
    const el = ref.current;
    if (followEnd && atEnd.current && el) {
      const pin = Math.max(0, total - height);
      el.scrollTop = pin;
      setTop(pin); // the scroll event follows in a browser; setting it here keeps the rows in step without waiting for one
    }
  }, [followEnd, total, height]);

  const prevOrigin = useRef(origin);
  useLayoutEffect(() => {
    const el = ref.current;
    const was = prevOrigin.current;
    prevOrigin.current = origin;
    if (!el || origin === undefined || was === undefined || origin === was || atEnd.current) return;
    const next = Math.max(0, el.scrollTop - (origin - was) * rowHeight);
    el.scrollTop = next;
    setTop(next);
  }, [origin, rowHeight]);

  const [bottom, setBottom] = useState(true);
  const start = Math.min(Math.max(0, count - 1), Math.floor(top / rowHeight));
  const end = Math.min(count, Math.ceil((top + height) / rowHeight));
  const report = useRef(onRange);
  report.current = onRange;
  useEffect(() => { report.current?.(start, end, bottom); }, [start, end, bottom]);

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
        setBottom(atEnd.current);
        setTop(el.scrollTop);
      }}
      {...aria}
    >
      <div style={{ height: total, position: "relative" }}>{rows}</div>
    </div>
  );
}
