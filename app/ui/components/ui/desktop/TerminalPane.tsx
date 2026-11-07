import { cn } from "../cn";
import { sanitizeLine } from "./ansi";
import { VirtualList } from "./VirtualList";

const ROW = 18;
const NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/**
 * Read-only view of a run session (or a worker stream). Lines are raw terminal output; they are sanitized (no links,
 * no clipboard or title sequences, no cursor control, no controls) and only the visible rows are sanitized and rendered.
 * `takenOver` is a label change only: keyboard input is the host's job (Cmd chords only), never this component's.
 */
export function TerminalPane({ lines, height = 360, takenOver, label, total, windowStart = 0, origin, onRange }: {
  /** The rows held in memory. With `total` they are a window onto a longer stream: row `windowStart + k` is `lines[k]`. */
  lines: readonly string[];
  height?: number;
  takenOver?: boolean;
  label?: string;
  /** Rows the scrollbar stands for (default: `lines.length`). Rows outside the held window render blank at the same height. */
  total?: number;
  windowStart?: number;
  /** Id of row 0 (the stream's first seq): when it advances the view stays on the same lines. */
  origin?: number;
  /** Rows in view (start inclusive, end exclusive) and whether the view is at the bottom. */
  onRange?: (start: number, end: number, atBottom: boolean) => void;
}) {
  const name = label ?? (takenOver ? "Run terminal, taken over" : "Run terminal, read only");
  return (
    <VirtualList
      className="v-terminal"
      role="log"
      aria-label={name}
      aria-live="off"
      tabIndex={0}
      count={Math.max(total ?? 0, windowStart + lines.length)}
      onRange={onRange}
      origin={origin}
      rowHeight={ROW}
      height={height}
      followEnd
      renderRow={(i) => {
        // Only the rows in view are sanitized (cost does not grow with the scrollback), so each line starts with plain
        // style: a color that a program leaves on across lines is not carried over.
        const text = lines[i - windowStart];
        if (text === undefined) return <span className="v-term-line" aria-hidden />;
        const line = sanitizeLine(text);
        return (
        <span className="v-term-line">
          {line.segments.map((s, k) => (
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
          {line.truncated && <span className="v-ansi-dim"> [line cut]</span>}
        </span>
        );
      }}
    />
  );
}
