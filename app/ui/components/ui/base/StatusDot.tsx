import { S } from "../../../strings";
import type { StatusKey } from "../../../strings";
import { cn } from "../cn";

// One SVG shape per status (DESIGN.md "Status vocabulary"): the shape and the text carry the meaning, color backs it up.
const RING = <circle cx="5" cy="5" r="3.5" fill="none" stroke="currentColor" strokeWidth="1.5" />;
const CHECK = <path d="M1.5 5.5l2.5 2.5 4.5-5.5" fill="none" stroke="currentColor" strokeWidth="1.6" />;
const CROSS = <path d="M1.8 1.8l6.4 6.4M8.2 1.8L1.8 8.2" fill="none" stroke="currentColor" strokeWidth="1.6" />;
const SHAPES: Record<StatusKey, JSX.Element> = {
  pending: RING,
  unknown: RING,
  ready: <circle cx="5" cy="5" r="3.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray="2 2" />,
  claimed: (
    <>
      <circle cx="5" cy="5" r="3.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
      <path d="M5 1.5a3.5 3.5 0 0 1 0 7z" fill="currentColor" />
    </>
  ),
  in_progress: <circle cx="5" cy="5" r="4" fill="currentColor" />,
  done: CHECK,
  pass: CHECK,
  blocked: <rect x="1.5" y="1.5" width="7" height="7" fill="currentColor" />,
  question: <path d="M5 .8L9.2 5 5 9.2.8 5z" fill="currentColor" />,
  failed: CROSS,
  fail: CROSS,
  needs_approval: <path d="M5 1L9.2 8.6H.8z" fill="currentColor" />,
};

/** Shape, color and text label together; never the dot alone. Pass `label` to override the default wording. */
export function StatusDot({ status, label, className }: { status: StatusKey; label?: string; className?: string }) {
  const text = label ?? S.status[status];
  return (
    <span className={cn("v-status-dot", `v-status--${status}`, className)}>
      <svg viewBox="0 0 10 10" aria-hidden="true" focusable="false">
        {SHAPES[status]}
      </svg>
      <span>{text}</span>
    </span>
  );
}
