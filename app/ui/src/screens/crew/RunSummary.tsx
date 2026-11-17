import { useState } from "react";
import { Button, Section, StatusDot, Text } from "../../../components/ui/base";
import type { StatusKey } from "../../../strings";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { CrewNodeDto } from "../../ipc";
import { useRoute } from "../../router";
import { ago } from "../when";
import { LiveFeed } from "./LiveFeed";
import { activeWorker, clock, focusWorker, workersIn } from "./crewLive";

const C = S.crew;

/** "3 minutes ago (17 Nov, 14:02)" for a time in ms. */
export const whenText = (ms: number, now = Date.now()) => `${ago(new Date(ms).toISOString(), now)} (${clock(ms)})`;

/**
 * A run that says it is working but is not: when it last moved, why it reads as stalled, and the one thing to do. A run
 * with no process is reviewed; a live run that went silent can be stopped (the same Stop run as Home).
 */
export function StallNote({ since, live, slug, runId }: { since: number | null; live: boolean; slug: string; runId: string }) {
  const [, go] = useRoute();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const stop = () => {
    setBusy(true);
    api.stopRun(runId).then(() => setErr(""), (e) => setErr(`${C.stopFailed} ${errorText(e)}`)).finally(() => setBusy(false));
  };
  return (
    <div className="v-crew__stall" role="group" aria-label={C.stalled}>
      <div>
        <p className="v-crew__stall-t">{since !== null ? C.stalledSince(clock(since), ago(new Date(since).toISOString())) : C.stalledNoTime}</p>
        <p className="v-crew__stall-why">{live ? C.stalledSilentWhy : C.stalledWhy}</p>
        {err && <p className="v-crew__stall-why" role="alert">{err}</p>}
      </div>
      {live
        ? <Button size="sm" variant="danger" busy={busy} onClick={stop}>{busy ? C.stopping : C.stopRun}</Button>
        : <Button size="sm" onClick={() => go("review", { slug })}>{C.review}</Button>}
    </div>
  );
}

export type Facts = {
  /** The status as the tree shows it. */
  shape: StatusKey; label: string; hint?: string;
  /** Set when the run has stalled: the last activity (ms), or null when no time is known. */
  stalled?: number | null;
  live: boolean; slug: string; runId: string;
  /** The last activity seen, ms, or null; and whether any worker reports activity right now. */
  lastMs: number | null; activeNow: boolean;
  /** A worker's readable name, and the title of the task it is on (null for none). */
  nameOf: (w: CrewNodeDto) => string; taskOf: (w: CrewNodeDto) => string | null;
  onActivity: (at: number) => void;
};

/** The Lead, the run or another group: what is running now, which worker, how long since anything happened, and the
 * active (else newest) worker's activity underneath. */
export function RunSummary({ scope, f }: { scope: CrewNodeDto; f: Facts }) {
  const ws = workersIn(scope);
  const active = activeWorker(scope);
  const shown = focusWorker(scope);
  const done = ws.filter((w) => w.status === "done").length;
  const now = active ? C.summary.working(f.nameOf(active), f.taskOf(active))
    : ws.length === 0 ? C.summary.noWorkers
    : done === ws.length ? C.summary.idleDone(ws.length)
    : C.summary.idle;
  const last = f.activeNow ? C.summary.activeNow : f.lastMs !== null ? whenText(f.lastMs) : C.summary.noneSeen;
  return (
    <section className="v-crew__pane v-crew__summary" aria-label={scope.role ?? scope.title}>
      <div className="v-crew__panehead">
        <Text as="h2" variant="headline-md">{scope.role ?? scope.title}</Text>
        <StatusDot status={f.shape} label={f.label} />
      </div>
      {f.stalled !== undefined && <StallNote since={f.stalled} live={f.live} slug={f.slug} runId={f.runId} />}
      <Section title={C.summary.label}>
        <p className="v-crew__now">{now}</p>
        {f.hint && f.stalled === undefined && <Text variant="body-sm" muted>{f.hint}</Text>}
        <dl className="v-crew__facts" aria-label={C.summary.label}>
          {shown && (<><dt>{active ? C.summary.worker : C.summary.latest}</dt><dd>{f.nameOf(shown)}</dd></>)}
          <dt>{C.summary.lastActivity}</dt><dd>{last}</dd>
          {ws.length > 0 && (<><dt>{C.summary.progress}</dt><dd>{C.workersDone(done, ws.length)}</dd></>)}
        </dl>
      </Section>
      {/* A worker the core runs itself writes no stream; its pane has its changes and controls. */}
      {shown && shown.id.startsWith("agent:") && (
        <Section title={C.summary.streamOf(f.nameOf(shown))}>
          <LiveFeed key={shown.id} stream={`w-${shown.id.slice("agent:".length)}`} live={f.live} onActivity={f.onActivity} />
        </Section>
      )}
    </section>
  );
}
