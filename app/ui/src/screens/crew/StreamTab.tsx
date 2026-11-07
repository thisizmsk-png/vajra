import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Section, Text } from "../../../components/ui/base";
import { TerminalPane } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import { EARLIER_PAGE, MAX_ROWS, RING, appendTail, emptyRing, prependEarlier } from "./streamRing";
import type { Ring } from "./streamRing";

const POLL_MS = 1000;
const EVENT_MIN_GAP_MS = 500;
/** Pages one load may read back to back (64 KiB each); more is picked up by the next poll or event. */
const MAX_DRAIN_PAGES = 64;

/** One worker's output: a bounded window that follows the tail, with older output fetched on request. */
export function StreamTab({ stream }: { stream: string }) {
  const [ring, setRing] = useState<Ring>(emptyRing);
  const [earlier, setEarlier] = useState<"idle" | "busy">("idle");
  /** The oldest line shown when a page came back empty: nothing is older than it. Rows evicted later move the window past it. */
  const [exhaustedAt, setExhaustedAt] = useState<number | null>(null);
  const [err, setErr] = useState("");
  const [pin, setPin] = useState(0);
  const ringRef = useRef<Ring>(emptyRing);
  const capRef = useRef(RING);
  const busy = useRef(false);
  /** Bumped by Jump to latest: a response that began before it belongs to the old window and is dropped. */
  const gen = useRef(0);
  const last = useRef(0);

  const put = (r: Ring) => { ringRef.current = r; setRing(r); };

  const load = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    const g = gen.current;
    last.current = Date.now();
    try {
      // The core returns the oldest lines after the cursor, one page at a time (64 KiB): keep reading until the cursor
      // reaches head_seq, so opening a worker or Jump to latest gets to the newest lines now, not one page per poll.
      for (let page = 0; page < MAX_DRAIN_PAGES; page++) {
        const t = await api.tail(stream, ringRef.current.last);
        if (g !== gen.current) return;
        setErr("");
        const before = ringRef.current.last;
        put(appendTail(ringRef.current, t, capRef.current));
        const r = ringRef.current;
        if (r.detached || r.last <= before || r.last >= t.head_seq) break;
      }
    } catch (e) {
      if (g === gen.current) setErr(errorText(e));
    } finally {
      if (g === gen.current) busy.current = false;
    }
  }, [stream]);

  useEffect(() => {
    void load();
    const poll = setInterval(() => { if (!document.hidden) void load(); }, POLL_MS);
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged((p) => {
      const streams = (p as { streams?: unknown } | null)?.streams;
      if (Array.isArray(streams) && !streams.includes(stream)) return;
      const wait = EVENT_MIN_GAP_MS - (Date.now() - last.current);
      if (wait <= 0) void load();
      else if (later === undefined) later = setTimeout(() => { later = undefined; void load(); }, wait);
    });
    return () => { clearInterval(poll); clearTimeout(later); off(); };
  }, [stream, load]);

  /** The EARLIER_PAGE lines just before the oldest row shown, fetched by seq so they are adjacent to the window whatever the spill's size. */
  const loadEarlier = async () => {
    const first = ringRef.current.rows.find((r) => r.seq !== null)?.seq;
    if (first == null) return;
    const g = gen.current;
    setEarlier("busy");
    try {
      const p = await api.readSpillBefore(stream, first, EARLIER_PAGE);
      if (g !== gen.current) return;
      capRef.current = Math.min(MAX_ROWS, capRef.current + EARLIER_PAGE);
      put(prependEarlier(ringRef.current, p.lines, capRef.current));
      if (p.lines.length === 0) setExhaustedAt(first);
      setEarlier("idle");
    } catch (e) {
      if (g !== gen.current) return;
      setErr(errorText(e));
      setEarlier("idle");
    }
  };

  /** Back to the live tail: forget the paged history and re-read the newest rows. */
  const jumpLatest = () => {
    gen.current += 1;
    busy.current = false;
    capRef.current = RING;
    setExhaustedAt(null);
    put(emptyRing);
    setEarlier("idle");
    setPin((n) => n + 1);
    void load();
  };

  const text = useMemo(() => ring.rows.map((r) => r.text), [ring]);
  // Older output may exist whenever the oldest line shown is not the first one: in the core's own buffer, on disk, or
  // just evicted from this window (by rows or bytes) even though nothing was spilled.
  const firstSeq = ring.rows.find((r) => r.seq !== null)?.seq;
  const canLoad = firstSeq != null && firstSeq > 1 && firstSeq !== exhaustedAt;
  return (
    <Section title={S.crew.stream}>
      {err && <div role="alert"><Text variant="body-sm">{S.crew.streamFailed} {err}</Text></div>}
      {ring.detached && <Text variant="body-sm" muted>{S.crew.historyView}</Text>}
      <div className="v-crew__actions">
        {canLoad && (
          <Button size="sm" onClick={() => void loadEarlier()} busy={earlier === "busy"}>
            {earlier === "busy" ? S.crew.loadingEarlier : S.crew.loadEarlier}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={jumpLatest}>{S.crew.jumpLatest}</Button>
      </div>
      {text.length === 0 && !err ? (
        <Text variant="body-sm" muted>{S.crew.noStream}</Text>
      ) : (
        <TerminalPane key={pin} lines={text} label={S.crew.streamLabel} height={360} />
      )}
    </Section>
  );
}
