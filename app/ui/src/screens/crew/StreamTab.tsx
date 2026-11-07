import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Section, Text } from "../../../components/ui/base";
import { TerminalPane } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { Line } from "../../ipc";
import { EARLIER_CAP, RING, appendTail, emptyRing, prependEarlier } from "./streamRing";
import type { Ring } from "./streamRing";

const POLL_MS = 1000;
const EVENT_MIN_GAP_MS = 500;
const SPILL_PAGE = 64 * 1024;
/** At most this many spill pages are fetched per request (4 MiB), whatever the spill holds. */
const MAX_SPILL_PAGES = 64;

type SpillPage = { lines: Line[]; next_offset: number; end: boolean };

/** One worker's output: a bounded window that follows the tail, with older output fetched on request. */
export function StreamTab({ stream }: { stream: string }) {
  const [ring, setRing] = useState<Ring>(emptyRing);
  const [spill, setSpill] = useState<{ start: number; end: number } | null>(null);
  const [earlier, setEarlier] = useState<"idle" | "busy" | "done">("idle");
  const [err, setErr] = useState("");
  const [pin, setPin] = useState(0);
  const ringRef = useRef<Ring>(emptyRing);
  const capRef = useRef(RING);
  const busy = useRef(false);
  const last = useRef(0);

  const put = (r: Ring) => { ringRef.current = r; setRing(r); };

  const load = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    last.current = Date.now();
    try {
      const t = await api.tail(stream, ringRef.current.last);
      setErr("");
      put(appendTail(ringRef.current, t, capRef.current));
      if (t.spill_start != null && t.spill_end != null) setSpill({ start: t.spill_start, end: t.spill_end });
    } catch (e) {
      setErr(errorText(e));
    } finally {
      busy.current = false;
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

  const loadEarlier = async () => {
    if (!spill) return;
    setEarlier("busy");
    try {
      let off = spill.start;
      let keep: Line[] = [];
      for (let i = 0; i < MAX_SPILL_PAGES; i++) {
        const p = (await api.readSpill(stream, off, SPILL_PAGE)) as unknown as SpillPage;
        keep = keep.concat(p.lines).slice(-EARLIER_CAP); // a rolling window: the lines just before the live window
        if (p.end) break;
        off = p.next_offset;
      }
      capRef.current = RING + EARLIER_CAP;
      put(prependEarlier(ringRef.current, keep, capRef.current));
      setEarlier("done");
    } catch (e) {
      setErr(errorText(e));
      setEarlier("idle");
    }
  };

  const text = useMemo(() => ring.rows.map((r) => r.text), [ring]);
  const canLoad = spill !== null && spill.end > spill.start && earlier !== "done";
  return (
    <Section title={S.crew.stream}>
      {err && <div role="alert"><Text variant="body-sm">{S.crew.streamFailed} {err}</Text></div>}
      <div className="v-crew__actions">
        {canLoad && (
          <Button size="sm" onClick={() => void loadEarlier()} busy={earlier === "busy"}>
            {earlier === "busy" ? S.crew.loadingEarlier : S.crew.loadEarlier}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => setPin((n) => n + 1)}>{S.crew.jumpLatest}</Button>
      </div>
      {text.length === 0 && !err ? (
        <Text variant="body-sm" muted>{S.crew.noStream}</Text>
      ) : (
        <TerminalPane key={pin} lines={text} label={S.crew.streamLabel} height={360} />
      )}
    </Section>
  );
}
