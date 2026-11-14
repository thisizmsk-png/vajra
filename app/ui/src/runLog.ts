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

/** Keep only the entries before `seq` (and the head with them): what a break or a shorter log no longer vouches for. */
function cut(v: RunLogView, seq: number): RunLogView {
  if (lastSeq(v) < seq) return v;
  return { ...v, entries: v.entries.filter((e) => e.seq < seq), headSeq: Math.min(v.headSeq, seq - 1) };
}

/** Apply one `run_log_since` page. A break drops the entries it covers, even ones held from before (a view kept from an
 *  earlier visit, for a log broken since). */
export function applyPage(v: RunLogView, page: { entries: RunLogEntry[]; head_seq: number; broken: RunLogBreak | null; io_error: string | null }): RunLogView {
  let next = extend(v, page.entries, page.head_seq);
  if (page.broken) next = cut(next, page.broken.seq);
  if (next.broken === page.broken && next.ioError === page.io_error) return next;
  return { ...next, broken: page.broken, ioError: page.io_error };
}

/** Apply one notice with no call. Returns the new view, and whether the view still lags the notice's head (a gap that
 *  only a `run_log_since` read can fill). */
export function applyNotice(v: RunLogView, n: RunLogNotice): { view: RunLogView; gap: boolean } {
  // The log's state comes with every notice, so a caught-up view that makes no call still learns the log stopped.
  const view = applyPage(v, { entries: n.entries, head_seq: n.seq, broken: n.broken ?? null, io_error: n.io_error ?? null });
  return { view, gap: lastSeq(view) < n.seq };
}

export type RunLogStats = { notices: number; calls: number };

/** Follow `runId`'s log: `onView` gets each new view. Returns the stop function; `stats` counts notices and calls. One read
 *  runs at a time; notices that arrive during it are applied at once, a stop they bring outlives the read's older page,
 *  and a gap they leave costs one more read after it. `onRead` hears how each read ended: `null` when it was answered
 *  (even with nothing new), else the error (a run with no log yet answers `not_found`). */
export function followRunLog(
  runId: string,
  onView: (v: RunLogView) => void,
  start: RunLogView = emptyRunLog,
  stats: RunLogStats = { notices: 0, calls: 0 },
  onRead?: (failed: unknown) => void,
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
        const asked = view;
        const page = await runLogSince(runId, lastSeq(view));
        if (stopped) return;
        // A head below what the view held when it asked: the log is shorter now (cut on disk since a kept view was
        // made). Entries past it are no longer in the log, so they go. (A notice applied meanwhile can only raise it.)
        if (page.head_seq < lastSeq(asked)) show(cut(view, page.head_seq + 1));
        const before = lastSeq(view);
        // A notice applied while this read was pending is newer than its page: keep the stop it brought, or the page
        // would hide it and a stopped writer may never send another notice.
        show(applyPage(view, {
          ...page,
          io_error: view.ioError !== asked.ioError ? view.ioError : page.io_error,
          broken: view.broken !== asked.broken ? view.broken : page.broken,
        }));
        // A page is capped in the core: read on while it moved the view and more is written.
        if (lastSeq(view) > before && lastSeq(view) < page.head_seq) again = true;
        if (!again) onRead?.(null);
      } while (again && !stopped);
    } catch (e) {
      // A failed read is retried on the next notice; the view keeps what it has.
      if (!stopped) onRead?.(e ?? "failed");
    } finally {
      reading = false;
    }
  };
  const off = onRunLog((n) => {
    if (stopped || n.run_id !== runId) return;
    stats.notices++;
    // Even a notice with nothing new is applied: it may carry the log's stop (an error or a break).
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
