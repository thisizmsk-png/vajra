import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Section, StatusDot, Text } from "../../../components/ui/base";
import { taskStatusKey, timelineStatusKey } from "../../statusMap";
import { VirtualList } from "../../../components/ui/desktop";
import { RunProvider } from "../ProviderControls";
import { ProviderLine } from "../ProviderLine";
import { parseWorkerProvider } from "../../workerProvider";
import { S } from "../../../strings";
import { api, errorText, isIpcError, onChanged } from "../../ipc";
import type { EvidenceDetail, EvidenceRow, RunOverview } from "../../ipc";
import { useProjects } from "../../projects/store";
import { useRoute } from "../../router";
import { Empty, Problem, problemDetail } from "../states";
import { EvidenceApproval } from "./EvidenceApproval";
import { Proof } from "./Proof";
import { RunLog } from "./RunLog";
import { useTaskActivity } from "./taskActivity";
import { clock, isWorking, runStalled } from "../crew/crewLive";
import { ago, parseWhen } from "../when";

const ORDER: Record<string, number> = { fail: 0, unverified: 1, checking: 2, not_run: 3, stale: 4, pass: 5 };
const BADGE: Record<string, "success" | "warning" | "danger" | "info" | undefined> = { pass: "success", fail: "danger", unverified: "danger", checking: "info", not_run: undefined, stale: "warning" };
const keyOf = (r: { task: string; criterion: string }) => `${r.task}\u0000${r.criterion}`;
const ROW_H = 36;
/** A task row is taller: its provider cell can carry a second line (a retry, or the provider it really ran on). */
const TASK_H = 48;
/** A task status in the vocabulary the rest of the app uses (shape, colour, word); one the app has no word for shows as
 * the core sent it, on the "Not run" ring. */
const taskStatus = (s: string) => {
  const k = taskStatusKey(s);
  return k ? <StatusDot status={k} /> : <StatusDot status="unknown" label={s} />;
};
/** One line: the provider the task ran on (icon, "Model · provider", errors), when the core says. */
const provider = (t: { provider?: unknown }) => { const p = parseWorkerProvider(t.provider); return p ? <ProviderLine p={p} compact /> : null; };
const GRAPH_H = 288;
const LIST_H = 360;

/** The id the core gives a run (`YYYYMMDD-HHMM`). Empty until the run has written its first progress note or task. */
const hasRunId = (id: string) => /^\d{8}-\d{4}$/.test(id);

const label = (s: string) => (S.run.status as Record<string, string>)[s] ?? S.run.status.not_run;

/** Evidence is the Run view's default (POS-1.1). Rows render through VirtualList (only the rows in view exist) and are plain data. */
export function Run() {
  const [route, go] = useRoute();
  const slug = route.params.slug ?? "";
  // The workflow of this run ended in error (for example an ask it refuses): the core ended the session and kept why.
  const [failure, setFailure] = useState<{ run_id: string; message: string } | null>(null);
  useEffect(() => {
    let live = true;
    setFailure(null);
    if (!slug) return;
    // The core lists a failure only for a campaign's CURRENT attempt (a resume keeps the campaign, so an earlier
    // attempt's failure is never in the list), for every engine.
    const look = () => void api.activeRuns().catch(() => null).then(() => api.runFailures()).then((r) => { if (live) setFailure(r.runs.find((x) => x.campaign === slug) ?? null); }, () => {});
    look();
    const off = onChanged(look);
    return () => { live = false; off(); };
  }, [slug]);
  const P = useProjects();
  const [rows, setRows] = useState<EvidenceRow[] | null>(null);
  const [err, setErr] = useState("");
  /** The core says there is no task list yet (`not_found`): a run still planning, not a failure. */
  const [notFound, setNotFound] = useState(false);
  /** Whether this slug is a run the app has started and that is still active. `not_found` only reads as "not ready" for
   *  such a run: for a stale or unknown campaign it stays an error (nothing is coming). */
  const [starting, setStarting] = useState<boolean | null>(null);
  // Unknown (null) until the first answer, and reset only when the slug changes: a not_found answer must never flip it
  // back to "not starting" while the question is asked again (that drew the error alert for a moment, every early run).
  useEffect(() => { setStarting(null); }, [slug]);
  useEffect(() => {
    let live = true;
    if (!slug) return;
    api.activeRuns().then(
      (a) => { if (live) setStarting((a.runs as { campaign?: string }[]).some((r) => r.campaign === slug)); },
      // A failed lookup proves nothing: it stays unknown (so not_found reads as "not ready"), and the next not_found change asks again.
      () => undefined,
    );
    return () => { live = false; };
  }, [slug, notFound]);
  // While unknown, a not_found reads as "not ready" (never as the alert); only a known "not starting" keeps the error.
  const notReady = notFound && starting !== false;
  /** Rows whose check has not run yet: the core checks a bounded amount per call, so ask again until none are left. */
  const [pending, setPending] = useState(0);
  /** The chosen criterion by identity (task + criterion), not list position: statuses change while polling and reorder the list. */
  const [sel, setSel] = useState<string | null>(null);
  const [detail, setDetail] = useState<EvidenceDetail | "failed" | null>(null);

  const [overview, setOverview] = useState<RunOverview | null>(null);
  const [overviewErr, setOverviewErr] = useState(false);

  // Each load takes a number and only the newest may set state, on its answer and on its failure. Checking the slug is
  // not enough: two loads of the same campaign overlap when events arrive while one is out, and the older must not land
  // last. A new campaign, or leaving the view, retires whatever is still out.
  const evSeq = useRef(0);
  const ovSeq = useRef(0);
  const load = useCallback(() => {
    if (!slug) return;
    const mine = ++evSeq.current;
    api.evidenceList(slug).then(
      (r) => { if (mine !== evSeq.current) return; setRows(r.rows); setPending(r.pending ?? 0); setErr(""); setNotFound(false); },
      (e) => { if (mine === evSeq.current) { setErr(problemDetail(e)); setNotFound(isIpcError(e) && e.error === "not_found"); } },
    );
  }, [slug]);
  // A new campaign starts empty: the previous campaign's rows, pending count, error and selection are not shown under it.
  useEffect(() => {
    setRows(null); setPending(0); setErr(""); setNotFound(false); setSel(null);
    return () => { evSeq.current++; };
  }, [slug]);
  const loadOverview = useCallback(() => {
    if (!slug) return;
    const mine = ++ovSeq.current;
    api.runOverview(slug).then(
      (o) => { if (mine === ovSeq.current) { setOverview(o); setOverviewErr(false); } },
      () => { if (mine === ovSeq.current) setOverviewErr(true); },
    );
  }, [slug]);
  useEffect(() => {
    setOverview(null); setOverviewErr(false);
    return () => { ovSeq.current++; };
  }, [slug]);
  useEffect(loadOverview, [loadOverview]);
  useEffect(() => (slug ? onChanged(loadOverview) : undefined), [slug, loadOverview]);
  useEffect(load, [load]);
  useEffect(() => (slug ? onChanged(load) : undefined), [slug, load]);
  useEffect(() => {
    if (!pending) return;
    const t = setTimeout(load, 1500);
    return () => clearTimeout(t);
  }, [pending, rows, load]);

  // Failing first; ties keep the campaign's own order.
  const sorted = useMemo(
    () => (rows ?? []).map((r, i) => [r, i] as const).sort((a, b) => (ORDER[a[0].status] ?? 9) - (ORDER[b[0].status] ?? 9) || a[1] - b[1]).map((x) => x[0]),
    [rows],
  );
  const passing = sorted.filter((r) => r.status === "pass").length;

  const picked = sel === null ? null : sorted.find((r) => keyOf(r) === sel) ?? null;
  const pickedTask = picked?.task;
  // The recorded commands and their output come from the core one task at a time, only for the criterion being looked at.
  useEffect(() => {
    setDetail(null);
    if (!slug || !pickedTask) return;
    let live = true;
    api.evidenceDetail(slug, pickedTask).then((d) => { if (live) setDetail(d); }, () => { if (live) setDetail("failed"); });
    return () => { live = false; };
  }, [slug, pickedTask]);

  // Is this run going? A process of this app runs it (live), or it says it is active but nothing runs it: Stalled, by
  // the Workers screen's rule. Working rows then read Stalled, and only a live run's rows read their worker's newest line.
  const runId = overview && hasRunId(overview.run_id) ? overview.run_id : "";
  const live = P.liveCampaigns ? P.liveCampaigns.includes(slug ?? "") : P.liveRuns.includes(runId) ? true : null;
  const updatedMs = parseWhen(P.campaigns.find((c) => c.slug === slug)?.updated ?? "");
  const lastMs = Number.isFinite(updatedMs) ? updatedMs : null;
  const working = useMemo(() => (overview?.graph.tasks ?? []).filter((t) => isWorking(t.status)).map((t) => t.local), [overview]);
  const stalled = runStalled({ running: overview?.timeline.status === "active" && working.length > 0, live, lastMs, now: Date.now() });
  const activity = useTaskActivity(slug ?? "", runId, working, live === true);

  // With no project list there is nothing to choose from: "choose one in History" would send the person to an empty
  // screen, so say what failed and offer the retry.
  if (!slug && P.state === "error") {
    return (
      <div className="v-home">
        <Text as="h1" variant="headline-lg">{S.run.title}</Text>
        <Problem title={S.frame.loadFailed} detail={P.error} retry={P.reload} />
      </div>
    );
  }
  if (!slug) {
    return (
      <div className="v-home">
        <h1 className="v-sr">{S.run.title}</h1>
        <div className="v-empty v-empty--pane">
          <Text as="h2" variant="headline-md">{S.run.emptyTitle}</Text>
          <Text variant="body" muted>{S.run.pick}</Text>
          <div className="v-empty__act"><Button size="sm" onClick={() => go("history")}>{S.states.openHistory}</Button></div>
        </div>
      </div>
    );
  }
  // The goal leads, as it does in History. The list is already loaded for the project column; until it names this run,
  // the title stays "Run", and the slug is always on the meta line.
  const goal = P.campaigns.find((c) => c.slug === slug)?.goal?.trim();
  return (
    <div className="v-home">
      <header className="v-run__head">
        {/* A goal can be up to 1000 characters with line breaks: the CSS clamps it to two lines, and the tooltip has all of it. */}
        <h1 className="v-text v-text--headline-lg v-run__name" title={goal || undefined}>{goal || S.run.title}</h1>
        <div className="v-run__meta">
          {goal && <span>{S.run.title}</span>}
          <code>{slug}</code>
        </div>
      </header>
      {/* The first run from the welcome: one line on what the evidence rows mean. It lives in the link, so it is gone on the next visit. */}
      {route.params.first === "1" && (
        <div className="v-run__first" role="note">
          <Text variant="body-sm">{S.welcome.done.note}</Text>
          <Button size="sm" onClick={() => go("launch")}>{S.welcome.done.another}</Button>
        </div>
      )}
      {failure && (
        <div className="v-run__first" role="alert">
          <Text variant="body-sm">{S.home.failedRun(failure.run_id)}. {failure.message}</Text>
          <Button size="sm" onClick={() => go("launch")}>{S.welcome.done.another}</Button>
        </div>
      )}
      <RunProvider slug={slug} />
      {/* Keyed by run, so another run's dialog or answer never shows under this one. */}
      {/* Early in a run the core has no run id yet (nothing in the progress notes, no tasks): there is nothing to approve
          and nothing to log, and asking with an empty id only produced "not a run id". */}
      {overview && hasRunId(overview.run_id) && <EvidenceApproval key={`approval-${overview.run_id}`} runId={overview.run_id} />}
      <Proof slug={slug} starting={starting} />
      <Section title={S.run.timeline}>
        {overviewErr && <div role="alert"><Text variant="body-sm">{S.run.overviewFailed}</Text></div>}
        {overview && (
          <>
            {stalled
              ? <StatusDot status="blocked" label={`${lastMs !== null ? S.crew.stalledSince(clock(lastMs), ago(new Date(lastMs).toISOString())) : S.crew.stalledNoTime} ${S.crew.stalledWhy}`} />
              : <StatusDot status={timelineStatusKey(overview.timeline.status)} label={S.run.runStatus[overview.timeline.status]} />}
            {/* Exactly one phase is current; each state is text, never colour alone (AC-4.1.1). */}
            <ol aria-label={S.run.timeline} className="v-run__phases">
              {overview.timeline.phases.map((p) => (
                <li key={p.name} data-state={p.state} aria-current={p.state === "current" ? "step" : undefined}>
                  {S.run.phases[p.name] ?? p.name} <Text as="span" variant="body-sm" muted>{S.run.phaseState[p.state]}{p.name === "Fix" && p.state !== "pending" && p.round ? `, ${S.run.fixRound(p.round, p.max_rounds)}` : ""}</Text>
                </li>
              ))}
            </ol>
          </>
        )}
      </Section>
      <Section title={S.run.graph}>
        {overview && overview.graph.tasks.length === 0 && <Text variant="body-sm" muted>{S.run.graphEmpty}</Text>}
        {overview && overview.graph.tasks.length > 0 && (
          <VirtualList
            role="list"
            aria-label={S.run.graph}
            count={overview.graph.tasks.length}
            rowHeight={TASK_H}
            height={GRAPH_H}
            renderRow={(i) => {
              const t = overview.graph.tasks[i];
              const open = () => go("crew", { slug, id: overview.run_id, focus: `task:${t.local}` });
              const waits = t.deps.length > 0 ? `${S.run.waitsFor} ${t.deps.map((d) => d.slice(d.indexOf(".") + 1)).join(", ")}` : "";
              const doing = stalled ? "" : activity.get(t.local) ?? "";
              return (
                <div role="listitem" className="v-run__row v-run__task" onKeyDown={(e) => { if (e.key === "Enter" && e.target === e.currentTarget) open(); }} tabIndex={0}>
                  <Text as="span" variant="body-sm" muted>{S.run.wave(t.wave)}</Text>
                  {/* The title opens the task's worker chat, like Open; the newest line says what a working one is doing. */}
                  <span className="v-run__title">
                    <button type="button" className="v-run__open" onClick={open} title={t.title ?? undefined}>{t.local} {t.title ?? ""}</button>
                    {doing && <span className="v-run__doing" title={doing}>{doing}</span>}
                  </span>
                  <span className="v-run__who">{provider(t) ?? t.owner ?? ""}</span>
                  {stalled && isWorking(t.status) ? <StatusDot status="blocked" label={S.crew.stalled} /> : taskStatus(t.status)}
                  {/* Always present, so the columns line up down the list. A long list is cut with an ellipsis; the tooltip has all of it. */}
                  <span className="v-run__deps" title={waits || undefined}><Text as="span" variant="body-sm" muted>{waits}</Text></span>
                  <Button size="sm" variant="ghost" onClick={open} aria-label={S.run.openChat(t.local)}>{S.run.openWorker}</Button>
                </div>
              );
            }}
          />
        )}
      </Section>
      {/* The run's own hash-chained log, live; keyed by run so a new run starts from its kept view or from 0. */}
      {overview && hasRunId(overview.run_id) && <RunLog key={`log-${overview.run_id}`} runId={overview.run_id} />}
      <Section title={S.run.evidence}>
        {err && !notReady && (
          <div role="alert">
            <Text variant="body-sm">{S.run.failed} {err}</Text>
            <Button size="sm" variant="ghost" onClick={load}>{S.states.retry}</Button>
          </div>
        )}
        {notReady && !rows && <Text variant="body-sm" muted>{S.run.notReady}</Text>}
        {rows && rows.length === 0 && <Text variant="body-sm" muted>{S.run.none}</Text>}
        {rows && rows.length > 0 && (
          <>
            {/* Green only when every criterion is a pass (POS-1.5). */}
            {passing === sorted.length
              ? <Badge status="success">{S.run.allPass}</Badge>
              : <Badge status="danger">{S.run.summary(passing, sorted.length)}</Badge>}
            <VirtualList
              role="list"
              aria-label={S.run.criteria}
              count={sorted.length}
              rowHeight={ROW_H}
              height={LIST_H}
              renderRow={(i) => {
                const r = sorted[i];
                return (
                  <div role="listitem" className="v-run__row">
                    <Button size="sm" variant="ghost" onClick={() => setSel(keyOf(r))}>{r.criterion}</Button>
                    <span>{r.task}</span>
                    <Badge status={BADGE[r.status]}>{label(r.status)}</Badge>
                  </div>
                );
              }}
            />
          </>
        )}
        {picked && (
          <dl className="v-crew__facts" role="group" aria-label={S.run.detail}>
            <dt>{S.run.criterion}</dt><dd>{picked.criterion}</dd>
            <dt>{S.run.taskCol}</dt><dd>{picked.task} {picked.title ?? ""}</dd>
            {picked.owner && (<><dt>{S.run.writtenBy}</dt><dd>{picked.owner}</dd></>)}
            <dt>{S.run.statusCol}</dt><dd>{label(picked.status)}</dd>
            {picked.path || picked.head ? (
              <>
                <dt>{S.run.commit}</dt><dd>{picked.head ? picked.head.slice(0, 7) : ""}</dd>
                <dt>{S.run.evidenceFile}</dt><dd>{picked.path ?? ""}</dd>
                <dt>{S.run.commandsRun}</dt>
                <dd>
                  {detail === null && <Text variant="body-sm" muted>…</Text>}
                  {detail === "failed" && <Text variant="body-sm">{S.run.detailFailed}</Text>}
                  {detail && detail !== "failed" && detail.truncated && <Text variant="body-sm" muted>{S.run.detailTruncated}</Text>}
                  {detail && detail !== "failed" && !detail.truncated && detail.commands.length === 0 && <Text variant="body-sm" muted>{S.run.noCommands}</Text>}
                  {detail && detail !== "failed" && detail.commands.map((c, k) => (
                    <div key={k} className="v-run__cmd">
                      {/* Text nodes only: output is untrusted and never rendered as markup. */}
                      <code>{c.cmd ?? ""}</code> <Badge status={c.passed ? "success" : "danger"}>{S.run.exit(c.exit_code)}</Badge>
                      {c.cmd && <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(c.cmd ?? "")}>{S.crew.copy(S.run.copyWhat)}</Button>}
                      {c.output_tail && <pre aria-label={S.run.output}>{c.output_tail}</pre>}
                    </div>
                  ))}
                </dd>
              </>
            ) : (<><dt>{S.run.evidenceCol}</dt><dd>{S.run.noEvidence}</dd></>)}
            {picked.report && (<><dt>{S.run.report}</dt><dd>{picked.report}</dd></>)}
            {detail && detail !== "failed" && detail.notes && detail.notes !== picked.report && (<><dt>{S.run.notes}</dt><dd>{detail.notes}</dd></>)}
          </dl>
        )}
      </Section>
    </div>
  );
}
