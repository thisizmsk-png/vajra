// Follow one core run's log by sequence number (W2). Each `run_log` notice carries the newly written entries, so a view
// that holds the seq the notice starts after applies them with no call at all. Only a gap (a missed notice, a burst
// bigger than one notice, or entries already past what the view holds) costs a call: one `run_log_since` from the last
// seq held. A WebView reload loses nothing: the follower starts with one read from its kept view (or from 0).

import { onRunLog, runLogSince, type RunLogBreak, type RunLogEntry, type RunLogNotice } from "./ipc";

export type RunLogView = { entries: RunLogEntry[]; headSeq: number; broken: RunLogBreak | null; ioError: string | null };

export const emptyRunLog: RunLogView = { entries: [], headSeq: 0, broken: null, ioError: null };

const lastSeq = (v: RunLogView) => (v.entries.length ? v.entries[v.entries.length - 1].seq : 0);

/** Append the entries that continue the view (seq = last + 1, in order); skip ones it holds; stop at the first gap. */
function extend(v: RunLogView, entries: RunLogEntry[], headSeq: number): RunLogView {
  let last = lastSeq(v);
  const add: RunLogEntry[] = [];
  for (const e of entries) {
    if (e.seq <= last) continue;
    if (e.seq !== last + 1) break;
    add.push(e);
    last = e.seq;
  }
  const head = Math.max(v.headSeq, headSeq);
  if (!add.length && head === v.headSeq) return v;
  return { ...v, entries: add.length ? [...v.entries, ...add] : v.entries, headSeq: head };
}

/** Apply one `run_log_since` page. */
export function applyPage(v: RunLogView, page: { entries: RunLogEntry[]; head_seq: number; broken: RunLogBreak | null; io_error: string | null }): RunLogView {
  const next = extend(v, page.entries, page.head_seq);
  if (next.broken === page.broken && next.ioError === page.io_error) return next;
  return { ...next, broken: page.broken, ioError: page.io_error };
}

/** Apply one notice with no call. Returns the new view, and whether the view still lags the notice's head (a gap that
 *  only a `run_log_since` read can fill). */
export function applyNotice(v: RunLogView, n: RunLogNotice): { view: RunLogView; gap: boolean } {
  const view = extend(v, n.entries, n.seq);
  return { view, gap: lastSeq(view) < n.seq };
}

export type RunLogStats = { notices: number; calls: number };

/** Follow `runId`'s log: `onView` gets each new view. Returns the stop function; `stats` counts notices and calls. One read
 *  runs at a time; notices that arrive during it are applied after it, and a remaining gap costs one more read. */
export function followRunLog(
  runId: string,
  onView: (v: RunLogView) => void,
  start: RunLogView = emptyRunLog,
  stats: RunLogStats = { notices: 0, calls: 0 },
): () => void {
  let view = start;
  let reading = false;
  let again = false;
  let stopped = false;
  const show = (next: RunLogView) => {
    if (next !== view) {
      view = next;
      onView(view);
    }
  };
  const read = async () => {
    if (reading) {
      again = true;
      return;
    }
    reading = true;
    try {
      do {
        again = false;
        stats.calls++;
        const page = await runLogSince(runId, lastSeq(view));
        if (stopped) return;
        const before = lastSeq(view);
        show(applyPage(view, page));
        // A page is capped in the core: read on while it moved the view and more is written.
        if (lastSeq(view) > before && lastSeq(view) < page.head_seq) again = true;
      } while (again && !stopped);
    } catch {
      // A failed read is retried on the next notice; the view keeps what it has.
    } finally {
      reading = false;
    }
  };
  const off = onRunLog((n) => {
    if (stopped || n.run_id !== runId) return;
    stats.notices++;
    if (n.seq <= lastSeq(view)) return;
    const { view: next, gap } = applyNotice(view, n);
    show(next);
    if (gap) void read();
  });
  void read();
  return () => {
    stopped = true;
    off();
  };
}
