import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Button, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { StreamWindow } from "../../ipc";
import { onChangedPaced, usePoll } from "../../projects/poll";
import { appendTail } from "./streamAppend";
import { RESULT_LINES, announceOf, feedItems } from "./crewLive";
import type { FeedItem } from "./crewLive";

const POLL_MS = 1000;
const EVENT_MIN_GAP_MS = 500;
/** The feed holds the newest rows only, whatever the stream's length: older ones are in Raw output and the core. */
export const FEED_ROWS = 300;
export const FEED_BYTES = 256 * 1024;
/** A screen reader hears at most one line per this long, about the newest entry. */
export const ANNOUNCE_MS = 10_000;
/** Within this many pixels of the end counts as at the end. */
const END_SLACK = 24;
const L = S.crew.live;

/**
 * One worker's activity as it happens: the agent's messages and its tool steps, newest last. It follows the live end and
 * scrolls with it; scrolling up pauses that (new lines are counted, not shown) until the reader scrolls back down or
 * picks Follow live. `onActivity` hears when the stream grows, with the time it was seen.
 */
export function LiveFeed({ stream, live = true, onActivity }: { stream: string; live?: boolean; onActivity?: (at: number) => void }) {
  const [win, setWin] = useState<StreamWindow | null>(null);
  const [err, setErr] = useState("");
  const [following, setFollowing] = useState(true);
  const [pending, setPending] = useState(0);
  const [said, setSaid] = useState("");
  const winRef = useRef<StreamWindow | null>(null);
  const followRef = useRef(true);
  const busy = useRef(false);
  const head = useRef<number | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const activity = useRef(onActivity);
  activity.current = onActivity;

  const seen = useCallback((h: number) => {
    if (head.current !== null && h > head.current) activity.current?.(Date.now());
    head.current = Math.max(head.current ?? h, h);
  }, []);

  /** False once the feed has closed (another tab or worker): a read still out then changes nothing. */
  const open = useRef(true);
  useEffect(() => { open.current = true; return () => { open.current = false; }; }, []);

  const refresh = useCallback(async () => {
    if (busy.current || !open.current) return;
    busy.current = true;
    try {
      const held = winRef.current;
      const last = held?.lines[held.lines.length - 1]?.seq;
      if (!followRef.current && held && last !== undefined) {
        // Paused: count what is new, hold what is shown.
        const t = await api.tail(stream, last, 1);
        if (!open.current) return;
        seen(t.head_seq);
        setPending(Math.max(0, t.head_seq - last));
        return;
      }
      let next: StreamWindow | null = null;
      if (held && last !== undefined) {
        try { next = appendTail(held, await api.tail(stream, last), FEED_ROWS, FEED_BYTES); } catch { next = null; }
      }
      next ??= await api.streamWindow(stream, null, FEED_ROWS, FEED_BYTES);
      if (!open.current) return;
      seen(next.head_seq);
      setErr("");
      setPending(0);
      if (next !== held) { winRef.current = next; setWin(next); }
    } catch (e) {
      if (open.current) setErr(errorText(e));
    } finally {
      busy.current = false;
    }
  }, [stream, seen]);

  useEffect(() => {
    void refresh();
    return onChangedPaced(EVENT_MIN_GAP_MS, () => void refresh(), (p) => {
      const streams = (p as { streams?: unknown } | null)?.streams;
      return !Array.isArray(streams) || streams.includes(stream);
    });
  }, [stream, refresh]);
  usePoll(() => void refresh(), POLL_MS, live);

  const items = useMemo(() => (win ? feedItems(win.lines) : []), [win]);

  // Following: keep the newest entry in view after every change (one write, before paint).
  useLayoutEffect(() => {
    const el = box.current;
    if (el && followRef.current) el.scrollTop = el.scrollHeight;
  }, [items]);

  // One polite line at most every ANNOUNCE_MS, and only while following: a busy worker never floods a screen reader.
  const lastSaid = useRef({ at: 0, seq: 0 });
  useEffect(() => {
    const newest = items[items.length - 1];
    if (!newest || !followRef.current) return;
    const now = Date.now();
    if (lastSaid.current.seq === 0) { lastSaid.current = { at: now, seq: newest.seq }; return; } // not the backlog on open
    if (newest.seq <= lastSaid.current.seq || now - lastSaid.current.at < ANNOUNCE_MS) return;
    lastSaid.current = { at: now, seq: newest.seq };
    setSaid(L.announce(announceOf(newest)));
  }, [items]);

  const follow = () => {
    followRef.current = true;
    setFollowing(true);
    const el = box.current;
    if (el) el.scrollTop = el.scrollHeight;
    void refresh();
  };
  const onScroll = () => {
    const el = box.current;
    if (!el) return;
    const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight <= END_SLACK;
    if (atEnd === followRef.current) return;
    if (atEnd) follow();
    else { followRef.current = false; setFollowing(false); }
  };

  return (
    <div className="v-feed">
      {err && <div role="alert"><Text variant="body-sm">{L.failed} {err}</Text></div>}
      {win?.gap && <Text variant="body-sm" muted>{L.gap}</Text>}
      <div ref={box} className="v-feed__scroll" role="log" aria-live="off" aria-label={L.label} tabIndex={0} onScroll={onScroll}>
        {items.length === 0 && !err ? <p className="v-feed__empty">{L.waiting}</p> : items.map((it) => <Entry key={it.seq} item={it} />)}
      </div>
      {!following && (
        <div className="v-feed__paused">
          <Text variant="body-sm" muted>{pending > 0 ? `${L.paused} ${L.newLines(pending)}` : L.paused}</Text>
          <Button size="sm" variant="ghost" onClick={follow}>{L.follow}</Button>
        </div>
      )}
      <p className="v-sr" role="status" aria-live="polite">{said}</p>
    </div>
  );
}

const Entry = memo(function Entry({ item }: { item: FeedItem }) {
  if (item.kind === "step") {
    const shown = item.result.slice(0, RESULT_LINES);
    const more = item.result.length - shown.length;
    return (
      <div className="v-feed__step">
        <p className="v-feed__steprow">
          <span className="v-feed__tool">{item.tool}</span>
          {item.detail && <span className="v-feed__detail">{item.detail}</span>}
        </p>
        {shown.length > 0 && (
          <pre className="v-feed__result">{shown.join("\n")}{more > 0 && `\n${L.more(more)}`}</pre>
        )}
      </div>
    );
  }
  if (item.kind === "say") return <p className="v-feed__say">{item.text}</p>;
  return (
    <div className={`v-feed__aside v-feed__aside--${item.kind}`}>
      <p className="v-feed__who">{item.kind === "prompt" ? L.prompt : L.note}</p>
      <p className="v-feed__text">{item.text}</p>
    </div>
  );
});
