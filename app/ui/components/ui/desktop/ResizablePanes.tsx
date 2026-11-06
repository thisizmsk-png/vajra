import { useRef, useState } from "react";
import type { PointerEvent, ReactNode } from "react";

const STEP = 16;

/**
 * Two panes with a separator between them. The first pane's size is clamped to `min`..`max` (px, along `orientation`).
 * The handle is `role="separator"` with value, min and max; arrows resize by 16 px, Home/End jump to the limits.
 */
export function ResizablePanes({
  orientation = "horizontal",
  initial,
  min,
  max,
  label,
  first,
  second,
}: {
  orientation?: "horizontal" | "vertical";
  initial: number;
  min: number;
  max: number;
  label: string;
  first: ReactNode;
  second: ReactNode;
}) {
  const [size, setSize] = useState(Math.min(max, Math.max(min, initial)));
  const drag = useRef<{ from: number; start: number } | null>(null);
  const horiz = orientation === "horizontal";
  const clamp = (n: number) => Math.min(max, Math.max(min, n));
  const down = (e: PointerEvent) => {
    drag.current = { from: horiz ? e.clientX : e.clientY, start: size };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  };
  const move = (e: PointerEvent) => {
    if (drag.current) setSize(clamp(drag.current.start + (horiz ? e.clientX : e.clientY) - drag.current.from));
  };
  const key = (e: React.KeyboardEvent) => {
    const back = horiz ? "ArrowLeft" : "ArrowUp";
    const fwd = horiz ? "ArrowRight" : "ArrowDown";
    const to = e.key === back ? clamp(size - STEP) : e.key === fwd ? clamp(size + STEP) : e.key === "Home" ? min : e.key === "End" ? max : null;
    if (to === null) return;
    e.preventDefault();
    setSize(to);
  };
  return (
    <div className={`v-panes v-panes--${orientation}`}>
      <div className="v-pane" style={horiz ? { width: size } : { height: size }}>
        {first}
      </div>
      <div
        role="separator"
        aria-label={label}
        aria-orientation={horiz ? "vertical" : "horizontal"}
        aria-valuenow={size}
        aria-valuemin={min}
        aria-valuemax={max}
        tabIndex={0}
        className="v-handle"
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={() => { drag.current = null; }}
        onKeyDown={key}
      />
      <div className="v-pane v-pane--grow">{second}</div>
    </div>
  );
}
