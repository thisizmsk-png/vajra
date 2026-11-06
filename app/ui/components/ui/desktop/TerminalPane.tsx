import { useMemo } from "react";
import { cn } from "../cn";
import { sanitizeLines } from "./ansi";
import { VirtualList } from "./VirtualList";

const ROW = 18;
const NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/**
 * Read-only view of a run session (or a worker stream). Lines are raw terminal output; they are sanitized (no links,
 * no clipboard or title sequences, no cursor control, no controls) and only the visible rows are rendered.
 * `takenOver` is a label change only: keyboard input is the host's job (Cmd chords only), never this component's.
 */
export function TerminalPane({ lines, height = 360, takenOver, label }: { lines: readonly string[]; height?: number; takenOver?: boolean; label?: string }) {
  const clean = useMemo(() => sanitizeLines(lines), [lines]);
  const name = label ?? (takenOver ? "Run terminal, taken over" : "Run terminal, read only");
  return (
    <VirtualList
      className="v-terminal"
      role="log"
      aria-label={name}
      aria-live="off"
      aria-readonly={!takenOver}
      tabIndex={0}
      count={clean.length}
      rowHeight={ROW}
      height={height}
      followEnd
      renderRow={(i) => (
        <span className="v-term-line">
          {clean[i].segments.map((s, k) => (
            <span
              key={k}
              className={cn(
                s.style.fg !== null && `v-ansi-${NAMES[s.style.fg % 8]}${s.style.fg >= 8 ? "-bright" : ""}`,
                s.style.bold && "v-ansi-bold",
                s.style.dim && "v-ansi-dim",
                s.style.italic && "v-ansi-italic",
                s.style.underline && "v-ansi-underline",
              )}
            >
              {s.text}
            </span>
          ))}
          {clean[i].truncated && <span className="v-ansi-dim"> [line cut]</span>}
        </span>
      )}
    />
  );
}
