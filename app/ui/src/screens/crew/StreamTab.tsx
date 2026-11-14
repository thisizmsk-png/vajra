import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Section, Text } from "../../../components/ui/base";
import { TerminalPane } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { StreamWindow } from "../../ipc";
import { onChangedPaced, usePoll } from "../../projects/poll";
import { appendTail } from "./streamAppend";

const POLL_MS = 1000;
const EVENT_MIN_GAP_MS = 500;
const HEIGHT = 360;
const ROW = 18;
const VISIBLE = HEIGHT / ROW;
/** Rows held on each side of the view. The window is `VISIBLE + 2 * MARGIN` rows, a few hundred at most, whatever the stream's length. */
const MARGIN = 100;
/** Fetch the next window once the view comes within this many rows of an edge that has more beyond it. */
const EDGE = MARGIN / 2;
const WINDOW_ROWS = VISIBLE + 2 * MARGIN;
/** A following pane appends this many times, then reads a whole window once, which also brings `first_seq` up to date. */
const RESYNC_EVERY = 30;

/** One worker's output: the core serves a window of rows and the view holds only that window. */
export function StreamTab({ stream, live = true }: { stream: string; live?: boolean }) {
  const [win, setWin] = useState<StreamWindow | null>(null);
  const [following, setFollowing] = useState(true);
  const [err, setErr] = useState("");
  const [pin, setPin] = useState(0);
  const winRef = useRef<StreamWindow | null>(null);
  const followRef = useRef(true);
  const view = useRef({ start: 0, end: 0 });
  const busy = useRef(false);
  /** Bumped by Jump to latest: a response that began before it belongs to the old window and is dropped. */
  const gen = useRef(0);
  /** The `from` the view most recently asked for (null = the tail). A response for any other `from` is stale. */
  const want = useRef<number | null | undefined>(undefined);
  const queued = useRef(false);
  /** The `from` of a scroll request that has not been answered yet: a background refresh re-asks for it, never for the old window's start. */
  const nav = useRef<number | null>(null);
  /** The last numeric request that was answered, and the first row held after it. Asking again for the same `from` while the held window has not moved is a loop (a lost prefix is answered with later rows and `gap`). */
  const answered = useRef<{ from: number; held0: number } | null>(null);
  /** The first row held after a request whose prefix the core had lost (it answered with later rows and `gap`). The poll re-reads from that held start; its answer carries no `gap` of its own, so it must not clear the notice or the guard above. */
  const lostHeld0 = useRef<number | null>(null);
  /** The latest `reconcile`, for `fetchWin` to re-check the view once an answer lands (an answer does not re-render the list's range). */
  const reconcileRef = useRef<() => void>(() => {});

  /** Appends since the last whole-window read. `first_seq` only moves when a whole window is read, so it is refreshed every `RESYNC_EVERY`. */
  const appended = useRef(0);

  /**
   * Following the live end with a window in hand: ask only for the lines after the last one held and append them (a poll
   * that finds nothing new costs a few bytes, not a window). Any miss, error or resync falls back to a whole window.
   */
  const read = useCallback(async (from: number | null): Promise<StreamWindow> => {
    const held = winRef.current;
    const last = held?.lines[held.lines.length - 1]?.seq;
    if (from === null && held && last !== undefined && appended.current < RESYNC_EVERY) {
      try {
        const next = appendTail(held, await api.tail(stream, last), WINDOW_ROWS);
        if (next) { appended.current += 1; return next; }
      } catch { /* a whole window below answers, or reports the failure itself */ }
    }
    appended.current = 0;
    return api.streamWindow(stream, from, WINDOW_ROWS);
  }, [stream]);

  const fetchWin = useCallback(async (from: number | null) => {
    want.current = from;
    if (busy.current) { queued.current = true; return; }
    busy.current = true;
    const g = gen.current;
    try {
      let w = await read(from);
      if (g !== gen.current || want.current !== from) return;
      setErr("");
      const first = w.lines[0]?.seq;
      // A poll re-reading the held start of a lost-prefix window: same rows, still short of what was asked, so still a gap.
      const rereadOfLost = from !== null && first !== undefined && lostHeld0.current === first && from === first;
      if (rereadOfLost) w = { ...w, gap: true };
      winRef.current = w;
      if (nav.current === from) nav.current = null;
      if (from !== null && first !== undefined && !rereadOfLost) {
        answered.current = { from, held0: first };
        lostHeld0.current = w.gap && from < first ? first : null;
      } else if (from === null || first === undefined) {
        // The tail (or an empty answer) replaced the held window: an earlier numeric answer no longer describes it, so a
        // later return to the same `from` must be asked again.
        answered.current = null;
        lostHeld0.current = null;
      }
      setWin(w);
      // The view may still lie outside what came back (a short, byte-capped window): look again with the new window.
      setTimeout(() => reconcileRef.current(), 0);
    } catch (e) {
      if (g === gen.current) setErr(errorText(e));
    } finally {
      if (g === gen.current) {
        busy.current = false;
        if (queued.current) { queued.current = false; void fetchWin(want.current ?? null); }
      }
    }
  }, [stream, read]);

  /** Re-read what the view is on: the tail while following, otherwise the rows from the held start. */
  const refresh = useCallback(() => {
    const w = winRef.current;
    void fetchWin(followRef.current || !w ? null : nav.current ?? w.lines[0]?.seq ?? w.first_seq);
  }, [fetchWin]);

  useEffect(() => {
    refresh();
    // Hidden window: nothing is drawn. The core keeps the log; one read on showing again catches up.
    return onChangedPaced(EVENT_MIN_GAP_MS, refresh, (p) => {
      const streams = (p as { streams?: unknown } | null)?.streams;
      return !Array.isArray(streams) || streams.includes(stream);
    });
  }, [stream, refresh]);
  // A live worker's stream is polled (output can outrun the events); a finished one only on events.
  usePoll(refresh, POLL_MS, live);

  /**
   * Ask for the rows around the view when it nears an edge of the held window that has more beyond it. Row i is seq `first_seq + i`.
   * The core may answer with fewer than `WINDOW_ROWS` (a window is also capped by bytes), so the rows kept before the view shrink
   * to what the last answer could hold on each side: a short window must still reach the visible rows.
   */
  const reconcile = useCallback(() => {
    const w = winRef.current;
    const { start, end } = view.current;
    if (followRef.current || !w || w.lines.length === 0) return;
    const held0 = w.lines[0].seq, held1 = w.lines[w.lines.length - 1].seq;
    const s0 = w.first_seq + start, s1 = w.first_seq + end - 1;
    const needLeft = !w.at_start && s0 - EDGE < held0;
    const needRight = !w.at_end && s1 + EDGE > held1;
    if (!needLeft && !needRight) return;
    const short = !w.at_end && w.lines.length < WINDOW_ROWS;
    const margin = short ? Math.min(MARGIN, Math.max(0, Math.floor((w.lines.length - VISIBLE) / 2))) : MARGIN;
    const from = Math.max(w.first_seq, s0 - margin);
    // An answered request that left the held window where it was made no progress: its rows are gone for good (the gap notice says so).
    if (answered.current?.from === from && answered.current.held0 === held0) return;
    if (from !== held0) { nav.current = from; void fetchWin(from); }
  }, [fetchWin]);
  reconcileRef.current = reconcile;

  const onRange = useCallback((start: number, end: number, atBottom: boolean) => {
    view.current = { start, end };
    followRef.current = atBottom;
    setFollowing(atBottom);
    reconcile();
  }, [reconcile]);

  /** Back to the live tail: forget the held window and read the newest rows. */
  const jumpLatest = () => {
    gen.current += 1;
    busy.current = false;
    queued.current = false;
    nav.current = null;
    answered.current = null;
    lostHeld0.current = null;
    followRef.current = true;
    winRef.current = null;
    setFollowing(true);
    setWin(null);
    setPin((n) => n + 1);
    void fetchWin(null);
  };

  const text = useMemo(() => (win ? win.lines.map((l) => l.text) : []), [win]);
  const total = win ? Math.max(0, win.head_seq - win.first_seq + 1) : 0;
  const windowStart = win && win.lines.length ? win.lines[0].seq - win.first_seq : 0;
  // Each line sits at its own seq (row = seq - first_seq), so a hole inside the window leaves blank rows and nothing shifts.
  const rowAt = useMemo(() => {
    if (!win) return undefined;
    const by = new Map<number, string>();
    for (const l of win.lines) by.set(l.seq, l.text);
    const first = win.first_seq;
    return (i: number) => by.get(first + i);
  }, [win]);
  return (
    <Section title={S.crew.stream}>
      {err && <div role="alert"><Text variant="body-sm">{S.crew.streamFailed} {err}</Text></div>}
      {win?.gap && <Text variant="body-sm" muted>{S.crew.windowGap}</Text>}
      {!following && <Text variant="body-sm" muted>{S.crew.historyView}</Text>}
      <div className="v-crew__actions">
        <Button size="sm" variant="ghost" onClick={jumpLatest}>{S.crew.jumpLatest}</Button>
      </div>
      {text.length === 0 && !err ? (
        <Text variant="body-sm" muted>{S.crew.noStream}</Text>
      ) : (
        <TerminalPane key={pin} lines={text} total={total} windowStart={windowStart} rowAt={rowAt} origin={win?.first_seq} onRange={onRange} label={S.crew.streamLabel} height={HEIGHT} />
      )}
    </Section>
  );
}
