import { useEffect, useRef, useState } from "react";
import { Badge, Button, Section, Text } from "../../../components/ui/base";
import { VirtualList } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { isIpcError, runLogVerify } from "../../ipc";
import type { RunLogEntry, RunLogStanding, RunLogVerify } from "../../ipc";
import { emptyRunLog, followRunLog } from "../../runLog";
import type { RunLogStats, RunLogView } from "../../runLog";
import "./runLog.css";

const L = S.run.log;
const ROW_H = 32;
const LIST_H = 320;

/** The views of the last few runs looked at, kept while the app is open: coming back to a run reads only what it
 *  missed (one `run_log_since` from the last seq held), never the whole log again. */
const KEEP = 8;
const kept = new Map<string, RunLogView>();
const keep = (runId: string, v: RunLogView) => {
  kept.delete(runId);
  kept.set(runId, v);
  while (kept.size > KEEP) kept.delete(kept.keys().next().value as string);
};
/** Tests start each case with nothing kept. */
export const forgetKeptRunLogs = () => kept.clear();

const str = (x: unknown) => (typeof x === "string" ? x : "");
const num = (x: unknown) => (typeof x === "number" && Number.isFinite(x) ? x : 0);

/** Who wrote an entry, as the core names its source: `core`, `verifier`, `lead`, `human` or `{worker: ticket}`. */
export function who(source: unknown): string {
  if (typeof source === "string") return L.who[source] ?? source;
  if (source && typeof source === "object" && "worker" in source) return L.worker(str((source as { worker: unknown }).worker));
  return "";
}

/** One line saying what happened. Every text field came from an agent or the core and is shown as text, never markup. */
export function what(e: RunLogEntry): string {
  const b = (e.body && typeof e.body === "object" ? e.body : {}) as Record<string, unknown>;
  const W = L.what;
  switch (e.kind) {
    case "run_started": return W.run_started(str(b.resumed_from) || null);
    case "plan_accepted": return W.plan_accepted(num(b.round), Array.isArray(b.tasks) ? b.tasks.length : 0);
    case "dispatched": return W.dispatched(str(b.task), str(b.seat));
    case "ticket_ended": return W.ticket_ended(str(b.task), str(b.outcome));
    case "integrated": return b.ok === true ? W.integrated(num(b.round), str(b.head) || null) : W.not_integrated(num(b.round), str(b.why) || null);
    case "checks_started": return W.checks_started(num(b.round));
    case "check_result": return W.check_result(b.passed === true, str(b.criterion));
    case "round_verdict": return W.round_verdict(num(b.round), str(b.verdict));
    case "question": return W.question(str(b.text));
    case "answer": return W.answer(str(b.text));
    case "status_set": return W.status_set(str(b.status), str(b.reason));
    case "lead_turn": return W.lead_turn(num(b.turns), num(b.tokens));
    case "receipt": {
      const t = (b.tokens && typeof b.tokens === "object" ? b.tokens : {}) as Record<string, unknown>;
      const tokens = num(t.fresh) + num(t.cache_write) + num(t.cache_read) + num(t.output);
      // `scope` is `run` or `task:<id>`.
      const scope = str(b.scope);
      return W.receipt(scope === "run" ? L.wholeRun : scope.replace(/^task:/, ""), str(b.model), tokens, `$${(num(b.usd_micros) / 1e6).toFixed(2)}`);
    }
    case "anchored": return W.anchored(num(b.seq));
    case "recovered": return W.recovered(num(b.dropped_bytes), num(b.at_seq));
    case "noop": return W.noop(str(b.what), str(b.why));
    default: return e.kind;
  }
}

const time = (ms: number) => new Date(ms).toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

/** The core's caveat is a clause ("hash-chained; until ..."); shown after a sentence, it starts with a capital. */
const sentence = (s: string) => (s ? `${s[0].toUpperCase()}${s.slice(1)}${/[.!?]$/.test(s) ? "" : "."}` : "");

/** How far the check vouches. "Signed" covers only the entries up to the last anchor the signed record holds; the
 *  ones after it are hash-chained but not signed yet, and the line says so. */
const reach = (c: RunLogVerify) =>
  c.standing === "signed" && c.signed ? L.signedTo(c.signed.signed_to, c.signed.unsigned_tail) : L.checkedTo(c.head_seq);

const tone: Record<RunLogStanding, "success" | "warning" | "danger"> = { signed: "success", unconfirmed: "warning", not_verified: "danger" };

type Read = "loading" | "ok" | "none" | "failed";

/** A core run's hash-chained log, live (W2). It follows the log by seq: each notice's entries are applied with no call,
 *  and only a gap, the first look or a return to the run costs one `run_log_since`. Rows are windowed. */
export function RunLog({ runId, stats }: { runId: string; stats?: RunLogStats }) {
  const [view, setView] = useState<RunLogView>(() => kept.get(runId) ?? emptyRunLog);
  const [read, setRead] = useState<Read>(() => (kept.has(runId) ? "ok" : "loading"));
  const [check, setCheck] = useState<RunLogVerify | "checking" | "failed" | null>(null);
  const checkSeq = useRef(0);

  useEffect(() => {
    const start = kept.get(runId) ?? emptyRunLog;
    setView(start);
    setRead(kept.has(runId) ? "ok" : "loading");
    setCheck(null);
    const stop = followRunLog(
      runId,
      (v) => { keep(runId, v); setView(v); setRead("ok"); },
      start,
      stats,
      (e) => {
        if (e === null) setRead("ok");
        // A failed read keeps what is shown; the next notice reads again. With nothing shown, say why.
        else setRead((r) => (r === "ok" ? r : isIpcError(e) && e.error === "not_found" ? "none" : "failed"));
      },
    );
    return () => { stop(); checkSeq.current++; };
  }, [runId, stats]);

  const verify = () => {
    const mine = ++checkSeq.current;
    setCheck("checking");
    runLogVerify(runId).then(
      (r) => { if (mine === checkSeq.current) setCheck(r); },
      () => { if (mine === checkSeq.current) setCheck("failed"); },
    );
  };

  const n = view.entries.length;
  return (
    <Section title={L.title}>
      <Text variant="body-sm" muted>{L.chain}</Text>
      {read === "loading" && n === 0 && <Text variant="body-sm" muted>{L.loading}</Text>}
      {read === "none" && n === 0 && <Text variant="body-sm" muted>{L.none}</Text>}
      {read === "failed" && n === 0 && <div role="alert"><Text variant="body-sm">{L.failed}</Text></div>}
      {view.broken && (
        <div role="alert"><Text variant="body-sm">{L.broken(view.broken.seq, L.why[view.broken.why] ?? view.broken.why)}</Text></div>
      )}
      {view.ioError && (
        <div role="alert">
          <Text variant="body-sm">{L.stopped}</Text>
          <Text variant="body-sm" muted>{view.ioError}</Text>
        </div>
      )}
      {n > 0 && (
        <>
          <div className="v-runlog__bar">
            <Text as="span" variant="body-sm" muted>{L.count(n)}</Text>
            <Button size="sm" variant="ghost" disabled={check === "checking"} onClick={verify}>{check === "checking" ? L.checking : L.check}</Button>
            <span role="status" className="v-runlog__standing">
              {check === "failed" && <Text as="span" variant="body-sm">{L.checkFailed}</Text>}
              {check && typeof check === "object" && (
                <>
                  <Badge status={tone[check.standing]}>{L.standing[check.standing]}</Badge>
                  <Text as="span" variant="body-sm" muted>{reach(check)} {sentence(check.caveat)}</Text>
                </>
              )}
            </span>
          </div>
          <VirtualList
            role="list"
            aria-label={L.entries}
            className="v-runlog"
            count={n}
            rowHeight={ROW_H}
            height={Math.min(LIST_H, n * ROW_H)}
            followEnd
            renderRow={(i) => {
              const e = view.entries[i];
              const line = what(e);
              return (
                <div role="listitem" className="v-runlog__row">
                  <Text as="span" variant="body-sm" muted numeric>{e.seq}</Text>
                  <Text as="span" variant="body-sm" muted numeric>{time(e.at_ms)}</Text>
                  <span className="v-runlog__who">{who(e.source)}</span>
                  <span className="v-runlog__what" title={line}>{line}</span>
                </div>
              );
            }}
          />
        </>
      )}
    </Section>
  );
}
