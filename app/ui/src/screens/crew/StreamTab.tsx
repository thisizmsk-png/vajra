import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Section, Text } from "../../../components/ui/base";
import { TerminalPane } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { StreamWindow } from "../../ipc";

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

/** One worker's output: the core serves a window of rows and the view holds only that window. */
export function StreamTab({ stream }: { stream: string }) {
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
  const last = useRef(0);

  const fetchWin = useCallback(async (from: number | null) => {
    want.current = from;
    if (busy.current) { queued.current = true; return; }
    busy.current = true;
    const g = gen.current;
    last.current = Date.now();
    try {
      const w = await api.streamWindow(stream, from, WINDOW_ROWS);
      if (g !== gen.current || want.current !== from) return;
      setErr("");
      winRef.current = w;
      setWin(w);
    } catch (e) {
      if (g === gen.current) setErr(errorText(e));
    } finally {
      if (g === gen.current) {
        busy.current = false;
        if (queued.current) { queued.current = false; void fetchWin(want.current ?? null); }
      }
    }
  }, [stream]);

  /** Re-read what the view is on: the tail while following, otherwise the rows from the held start. */
  const refresh = useCallback(() => {
    const w = winRef.current;
    void fetchWin(followRef.current || !w ? null : w.lines[0]?.seq ?? w.first_seq);
  }, [fetchWin]);

  useEffect(() => {
    refresh();
    const poll = setInterval(() => { if (!document.hidden) refresh(); }, POLL_MS);
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged((p) => {
      const streams = (p as { streams?: unknown } | null)?.streams;
      if (Array.isArray(streams) && !streams.includes(stream)) return;
      const wait = EVENT_MIN_GAP_MS - (Date.now() - last.current);
      if (wait <= 0) refresh();
      else if (later === undefined) later = setTimeout(() => { later = undefined; refresh(); }, wait);
    });
    return () => { clearInterval(poll); clearTimeout(later); off(); };
  }, [stream, refresh]);

  const onRange = useCallback((start: number, end: number, atBottom: boolean) => {
    view.current = { start, end };
    followRef.current = atBottom;
    setFollowing(atBottom);
    const w = winRef.current;
    if (atBottom || !w || w.lines.length === 0) return;
    // Row i is seq `first_seq + i`. Fetch when the view nears a window edge that has more beyond it.
    const held0 = w.lines[0].seq, held1 = w.lines[w.lines.length - 1].seq;
    const s0 = w.first_seq + start, s1 = w.first_seq + end - 1;
    const needLeft = !w.at_start && s0 - EDGE < held0;
    const needRight = !w.at_end && s1 + EDGE > held1;
    if (!needLeft && !needRight) return;
    const from = Math.max(w.first_seq, s0 - MARGIN);
    if (from !== held0) void fetchWin(from);
  }, [fetchWin]);

  /** Back to the live tail: forget the held window and read the newest rows. */
  const jumpLatest = () => {
    gen.current += 1;
    busy.current = false;
    queued.current = false;
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
