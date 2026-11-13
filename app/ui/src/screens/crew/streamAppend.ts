import type { StreamWindow, Tail } from "../../ipc";

/** The most text one pane keeps while it follows the live end. Same as the core's default for a window. */
export const PANE_BYTES = 1024 * 1024;

const size = (lines: { text: string }[]) => lines.reduce((n, l) => n + l.text.length, 0);

/**
 * A pane that follows the live end holds its window and asks only for the lines after the last one it has, then appends
 * them. Returns the new window, `held` itself when nothing new arrived, or null when the pane must read a whole window
 * instead: the core lost lines after ours (`gap`), the stream restarted, or the answer does not continue the window.
 *
 * The pane is capped: the oldest rows drop off the front until at most `rows` rows and `PANE_BYTES` of text remain (the
 * newest row always stays). Rows that dropped are still in the core and are read again when the reader scrolls back.
 */
export function appendTail(held: StreamWindow, tail: Tail, rows: number, maxBytes = PANE_BYTES): StreamWindow | null {
  const last = held.lines[held.lines.length - 1]?.seq;
  if (last === undefined || held.gap || !held.at_end) return null;
  if (tail.gap || tail.head_seq < last) return null;
  if (tail.lines.length === 0) return tail.head_seq === last ? held : null;
  let prev = last;
  for (const l of tail.lines) {
    if (l.seq <= prev) return null;
    prev = l.seq;
  }
  // The core stops a tail at its byte budget but still reports the stream's real head: lines that stop short of it are a
  // partial answer, and publishing the head over them would leave the newest rows blank. Read a whole window instead.
  if (prev < tail.head_seq) return null;
  const lines = held.lines.concat(tail.lines);
  let drop = Math.max(0, lines.length - rows);
  let bytes = size(lines.slice(drop));
  while (bytes > maxBytes && drop < lines.length - 1) bytes -= lines[drop++].text.length;
  const kept = drop ? lines.slice(drop) : lines;
  return {
    lines: kept,
    head_seq: tail.head_seq,
    first_seq: held.first_seq,
    at_start: held.at_start && drop === 0,
    at_end: kept[kept.length - 1].seq >= tail.head_seq,
    gap: false,
  };
}
