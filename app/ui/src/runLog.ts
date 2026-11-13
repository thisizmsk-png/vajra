// Follow one core run's log by sequence number (W2): the UI asks only for "entries after N" and appends them. A missed
// notice or a WebView reload loses nothing, because the next read starts from the last seq this view holds. There is no
// "refetch everything": a notice past what the view holds asks for the rest, in pages.

import { onRunLog, runLogSince, type RunLogBreak, type RunLogEntry } from "./ipc";

export type RunLogView = { entries: RunLogEntry[]; headSeq: number; broken: RunLogBreak | null; ioError: string | null };

export const emptyRunLog: RunLogView = { entries: [], headSeq: 0, broken: null, ioError: null };

/** Apply one page: keep only the entries that continue the view (seq = last + 1, in order). A page that doesn't
 *  continue it (a gap, or entries already held) adds nothing past the first mismatch, so the next read re-asks. */
export function applyPage(v: RunLogView, page: { entries: RunLogEntry[]; head_seq: number; broken: RunLogBreak | null; io_error: string | null }): RunLogView {
  let last = v.entries.length ? v.entries[v.entries.length - 1].seq : 0;
  const add: RunLogEntry[] = [];
  for (const e of page.entries) {
    if (e.seq <= last) continue;
    if (e.seq !== last + 1) break;
    add.push(e);
    last = e.seq;
  }
  return { entries: add.length ? [...v.entries, ...add] : v.entries, headSeq: Math.max(v.headSeq, page.head_seq), broken: page.broken, ioError: page.io_error };
}

const lastSeq = (v: RunLogView) => (v.entries.length ? v.entries[v.entries.length - 1].seq : 0);

/** Follow `runId`'s log: `onView` gets each new view. Returns the stop function. One read runs at a time; a notice that
 *  arrives during it is served by one more read after it, never by a second read in parallel. */
export function followRunLog(runId: string, onView: (v: RunLogView) => void, start: RunLogView = emptyRunLog): () => void {
  let view = start;
  let reading = false;
  let again = false;
  let stopped = false;
  const read = async () => {
    if (reading) {
      again = true;
      return;
    }
    reading = true;
    try {
      do {
        again = false;
        const page = await runLogSince(runId, lastSeq(view));
        if (stopped) return;
        const next = applyPage(view, page);
        const grew = next.entries.length > view.entries.length;
        if (next !== view) {
          view = next;
          onView(view);
        }
        // More is written than this page carried: read on (a page is capped in the core).
        if (grew && lastSeq(view) < page.head_seq) again = true;
      } while (again && !stopped);
    } catch {
      // A failed read is retried on the next notice; the view keeps what it has.
    } finally {
      reading = false;
    }
  };
  const off = onRunLog((n) => {
    if (n.run_id === runId && n.seq > lastSeq(view)) void read();
  });
  void read();
  return () => {
    stopped = true;
    off();
  };
}
