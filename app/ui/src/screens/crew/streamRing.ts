// A bounded window onto a worker's output. The renderer never holds more than `cap` rows however much the worker
// prints: older rows are dropped from memory and, if the core still has them, fetched again on request ("Load earlier").
import { S } from "../../../strings";
import type { Line, Tail } from "../../ipc";

/** Rows kept for live output. */
export const RING = 2000;
/** Lines fetched by one "Load earlier"; the row cap grows by this much per click. */
export const EARLIER_PAGE = 1000;
/** The most rows ever held, however far back the user goes. */
export const MAX_ROWS = RING + 19 * EARLIER_PAGE;

/** `seq` is null for a marker row (output that was skipped), which is never a real line. */
export type Row = { seq: number | null; text: string };
/** `detached` is set once paging backward has pushed the newest rows out of a full window: live output is then not
 * appended (the window is a stretch of history) until the view is reset to the tail. */
export type Ring = { rows: Row[]; last: number; detached?: boolean };
export const emptyRing: Ring = { rows: [], last: 0 };

const skipped = (n: number): Row => ({ seq: null, text: `— ${n === 1 ? S.crew.skippedOne : S.crew.skippedMany.replace("{n}", String(n))} —` });
const trim = (rows: Row[], cap: number) => (rows.length > cap ? rows.slice(rows.length - cap) : rows);

/** Add the lines of a tail that are newer than the last seen one. A hole between them is marked, not hidden. */
export function appendTail(r: Ring, t: Pick<Tail, "lines" | "gap">, cap: number): Ring {
  if (r.detached) return r;
  const fresh = t.lines.filter((l) => l.seq > r.last);
  if (!fresh.length) return r;
  const rows = r.rows.slice();
  if (t.gap && r.last > 0 && fresh[0].seq > r.last + 1) rows.push(skipped(fresh[0].seq - r.last - 1));
  for (const l of fresh) rows.push({ seq: l.seq, text: l.text });
  return { rows: trim(rows, cap), last: fresh[fresh.length - 1].seq };
}

/** Put older lines (from the spill) before the window. Lines that overlap the window are ignored. Past `cap` the
 * window slides: the newest rows are dropped instead of the oldest, so history stays reachable all the way back. */
export function prependEarlier(r: Ring, earlier: Line[], cap: number): Ring {
  const first = r.rows.find((x) => x.seq !== null)?.seq ?? Number.POSITIVE_INFINITY;
  const older = earlier.filter((l) => l.seq < first).sort((a, b) => a.seq - b.seq);
  if (!older.length) return r;
  const rows: Row[] = older.map((l) => ({ seq: l.seq, text: l.text }));
  const hole = Number.isFinite(first) ? first - older[older.length - 1].seq - 1 : 0;
  if (hole > 0) rows.push(skipped(hole));
  const all = rows.concat(r.rows);
  if (all.filter((x) => x.seq !== null).length <= cap) return { rows: all, last: r.last, detached: r.detached };
  const kept = all.slice(0, cap);
  const tail = [...kept].reverse().find((x) => x.seq !== null)?.seq ?? r.last;
  return { rows: kept, last: tail, detached: true };
}
