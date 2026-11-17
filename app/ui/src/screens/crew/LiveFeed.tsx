import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Text } from "../../../components/ui/base";
import { DiffView } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { StreamWindow } from "../../ipc";
import { onChangedPaced, usePoll } from "../../projects/poll";
import { appendTail } from "./streamAppend";
import { RESULT_LINES, announceOf } from "./crewLive";
import { WRITE_TOOLS, chatItems } from "./chat";
import type { ChatItem, Delivery, Sent } from "./chat";

const POLL_MS = 1000;
const EVENT_MIN_GAP_MS = 500;
/** The feed holds the newest rows only, whatever the stream's length: older ones are in Raw output and the core. */
export const FEED_ROWS = 300;
export const FEED_BYTES = 256 * 1024;
/** A screen reader hears at most one line per this long, about the newest entry. */
export const ANNOUNCE_MS = 10_000;
/** Within this many pixels of the end counts as at the end. */
const END_SLACK = 24;
/** Height of a step's inline diff. */
const DIFF_HEIGHT = 216;
const L = S.crew.live;

export type FeedProps = {
  stream: string;
  live?: boolean;
  onActivity?: (at: number) => void;
  /** Messages the human sent this worker from the app, placed in the thread with where each is. */
  sent?: readonly Sent[];
  /** The worker has ended: a sent message that never reached it says so. */
  finished?: boolean;
  /** The worker's change to one file (a unified diff part, "" for none), read when a step that wrote it is opened. */
  diffOf?: (path: string) => Promise<string>;
};

/**
 * One worker as a conversation: its own sentences, the task the Lead gave it, the messages relayed to it, and its tool
 * steps as compact rows that open to their output (a test run says how many passed and failed; an edit opens to its
 * diff). It follows the live end and scrolls with it; scrolling up pauses that (new lines are counted, not shown) until
 * the reader scrolls back down or picks Follow live. `onActivity` hears when the stream grows, with the time it was seen.
 */
export function LiveFeed({ stream, live = true, onActivity, sent, finished = false, diffOf }: FeedProps) {
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

  const items = useMemo(() => chatItems(win?.lines ?? [], sent, finished), [win, sent, finished]);

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
        {items.length === 0 && !err ? <p className="v-feed__empty">{L.waiting}</p> : items.map((it) => <Entry key={it.seq} item={it} diffOf={diffOf} />)}
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

const DELIVERY_BADGE: Record<Delivery, "success" | "warning" | "danger" | "info" | undefined> = { sending: undefined, sent: "info", seen: "success", failed: "danger", ended: "warning" };

/** Where a message is, in words; the badge colour is never the only signal. */
function DeliveryLine({ state, error }: { state: Delivery; error?: string }) {
  return (
    <span className="v-chat__state">
      <Badge status={DELIVERY_BADGE[state]}>{L.delivery[state]}</Badge>
      {state === "sent" && <span>{L.deliveryHint}</span>}
      {state === "failed" && error && <span>{error}</span>}
    </span>
  );
}

/** A step's file change, read when the row is opened and not before. */
function StepDiff({ path, diffOf }: { path: string; diffOf: (path: string) => Promise<string> }) {
  const [d, setD] = useState<string | { err: string } | null>(null);
  useEffect(() => {
    let live = true;
    diffOf(path).then((x) => live && setD(x), (e) => live && setD({ err: errorText(e) }));
    return () => { live = false; };
  }, [path, diffOf]);
  if (d === null) return <Text variant="body-sm" muted>{S.crew.loading}</Text>;
  if (typeof d !== "string") return <Text variant="body-sm" muted>{d.err}</Text>;
  if (!d) return <Text variant="body-sm" muted>{L.noFileDiff}</Text>;
  return <DiffView diff={d} height={DIFF_HEIGHT} />;
}

function Step({ item, diffOf }: { item: Extract<ChatItem, { kind: "step" }>; diffOf?: (path: string) => Promise<string> }) {
  const [opened, setOpened] = useState(false);
  const shown = item.result.slice(0, RESULT_LINES);
  const more = item.result.length - shown.length;
  const showsDiff = !!item.path && WRITE_TOOLS.has(item.tool) && !!diffOf;
  const t = item.test;
  const canOpen = shown.length > 0 || showsDiff || !!item.path;
  const row = (
    <>
      <span className="v-feed__tool">{item.tool}</span>
      {item.detail && <span className={item.path ? "v-feed__detail v-feed__path" : "v-feed__detail"}>{item.detail}</span>}
      {t && <Badge status={t.failed > 0 ? "danger" : "success"}>{L.tests(t.passed, t.failed)}</Badge>}
    </>
  );
  if (!canOpen) return <div className="v-feed__step"><p className="v-feed__steprow">{row}</p></div>;
  return (
    <details className="v-feed__step" onToggle={(e) => setOpened((e.currentTarget as HTMLDetailsElement).open)}>
      <summary className="v-feed__steprow">{row}</summary>
      {shown.length > 0 && <pre className="v-feed__result">{shown.join("\n")}{more > 0 && `\n${L.more(more)}`}</pre>}
      {item.path && (
        <span className="v-feed__stepacts">
          <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(item.path!)}>{S.crew.copy(L.path)}</Button>
        </span>
      )}
      {opened && showsDiff && <StepDiff path={item.path!} diffOf={diffOf!} />}
    </details>
  );
}

const Entry = memo(function Entry({ item, diffOf }: { item: ChatItem; diffOf?: (path: string) => Promise<string> }) {
  if (item.kind === "step") return <Step item={item} diffOf={diffOf} />;
  if (item.kind === "agent") return <p className="v-feed__say">{item.text}</p>;
  if (item.kind === "note") {
    return (
      <div className="v-feed__aside v-feed__aside--note">
        <p className="v-feed__who">{L.note}</p>
        <p className="v-feed__text">{item.text}</p>
      </div>
    );
  }
  const who = item.kind === "lead" ? L.fromLeadTask : item.from === "lead" ? (item.redirect ? L.leadRedirect : L.fromLead) : item.redirect ? L.youRedirect : L.fromYou;
  const mine = item.kind === "steer" && item.from === "you";
  return (
    <article className={mine ? "v-chat__msg v-chat__msg--you" : "v-chat__msg"}>
      <h3 className="v-feed__who">{who}</h3>
      <p className="v-feed__text">{item.text}</p>
      {item.kind === "steer" && item.state && <DeliveryLine state={item.state} error={item.error} />}
    </article>
  );
});
