import { useCallback, useEffect, useRef, useState } from "react";
import { Button, StatusDot, Text } from "../../../components/ui/base";
import { ActionBar, CrewTree } from "../../../components/ui/desktop";
import type { CrewNode } from "../../../components/ui/desktop";
import { RunProvider } from "../ProviderControls";
import { iconOf } from "../ProviderLine";
import { parseWorkerProvider } from "../../workerProvider";
import { S } from "../../../strings";
import type { StatusKey } from "../../../strings";
import { api } from "../../ipc";
import { Empty, LoadingRows, Problem, problemDetail } from "../states";
import type { CrewNodeDto } from "../../ipc";
import { useCommands } from "../../palette";
import { useRoute } from "../../router";
import { onChangedPaced, usePoll } from "../../projects/poll";
import { useProjects } from "../../projects/store";
import { WorkerPane } from "./WorkerPane";
import { CoreWorkerPane } from "./CoreWorkerPane";
import { RunSummary, StallNote } from "./RunSummary";
import type { Facts } from "./RunSummary";
import { initialPick, isWorkerId, isWorking, stalledSince, workersIn } from "./crewLive";
import { parseWhen } from "../when";

/** A status or event is re-read at least this often: a tasks.json edit raises no event (AC-2.1.2: within 2 s). */
const POLL_MS = 2000;
/** Changed events arrive in bursts; one re-read per window is enough. */
const EVENT_MIN_GAP_MS = 500;

type Target = { slug: string; runId: string };
const STATUSES = new Set<string>(["pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed", "unknown"]);
const statusOf = (s: string): StatusKey => (STATUSES.has(s) ? (s as StatusKey) : "unknown");

/** Whether a node, or anything under it, still says it is working. */
const anyWorking = (n: CrewNodeDto): boolean => isWorking(n.status) || n.children.some(anyWorking);

/** How one node reads: the dot, the words next to it and, for a node that says nothing, why. */
type Shown = { shape: StatusKey; label: string; hint?: string };

/**
 * The text next to the dot: the status, plus any overlay. A node nothing reports on says "No live updates" (the hint says
 * why); a node that says it is working in a run that has stalled says "Stalled" instead of a blue In progress.
 */
function statusShown(n: CrewNodeDto, stalled: boolean): Shown {
  const key = statusOf(n.status);
  const extra = (n.overlays as unknown[]).filter((o): o is string => typeof o === "string").map((o) => S.crew.overlay[o] ?? o);
  const join = (base: string) => [base, ...extra].join(", ");
  if (stalled && anyWorking(n)) return { shape: "blocked", label: join(S.crew.stalled), hint: S.crew.stalledHint };
  if (key !== "unknown") return { shape: key, label: join(S.status[key]) };
  const hint = n.kind === "run" || n.kind === "lead" ? S.crew.noLiveWhy.run : isWorkerId(n.id) ? S.crew.noLiveWhy.worker : S.crew.noLiveWhy.group;
  return { shape: "unknown", label: join(S.crew.unknownSignal), hint };
}
/** The label alone, for the palette and the action bar outside a run's context. */
const statusLabel = (n: CrewNodeDto) => statusShown(n, false).label;

function toNode(n: CrewNodeDto, stalled: boolean): CrewNode {
  const p = parseWorkerProvider(n.provider);
  const s = statusShown(n, stalled);
  return { id: n.id, label: n.title, status: s.shape, statusLabel: s.label, statusHint: s.hint, icon: p ? iconOf(p) : undefined, children: n.children.map((c) => toNode(c, stalled)) };
}

const find = (n: CrewNodeDto, id: string): CrewNodeDto | null => {
  if (n.id === id) return n;
  for (const c of n.children) {
    const f = find(c, id);
    if (f) return f;
  }
  return null;
};
/** The nearest ancestor of `id` that is a task group (`task:<tid>`), whose run-local id the worker's task is. */
const taskGroupOf = (n: CrewNodeDto, id: string, task: string | null = null): string | null | undefined => {
  if (n.id === id) return task;
  for (const c of n.children) {
    const f = taskGroupOf(c, id, n.id.startsWith("task:") ? n.id.slice("task:".length) : task);
    if (f !== undefined) return f;
  }
  return undefined;
};
/** A worker the core runs itself (ADR-16 16.7): its id is the full `core:<ticket>` reference its controls act on. */
const isCore = (id: string) => id.startsWith("core:");
/** Every worker in the tree (a run's subagents and the core's own), in tree order. */
const agents = (n: CrewNodeDto) => workersIn(n);
/** Re-render this often so "5 minutes ago" and the silent-run check stay current. */
const CLOCK_MS = 30_000;
/** Ids from the root down to `id`, or null. */
const pathTo = (n: CrewNodeDto, id: string): string[] | null => {
  if (n.id === id) return [n.id];
  for (const c of n.children) {
    const p = pathTo(c, id);
    if (p) return [n.id, ...p];
  }
  return null;
};
const withChildren = (n: CrewNodeDto, out: string[] = []): string[] => {
  if (n.children.length) out.push(n.id);
  n.children.forEach((c) => withChildren(c, out));
  return out;
};

export function Crew() {
  const [route, go] = useRoute();
  const { liveRuns, campaigns } = useProjects();
  const routed = route.params.slug && route.params.id ? { slug: route.params.slug, runId: route.params.id } : undefined;
  const [target, setTarget] = useState<Target | null | undefined>(routed);
  const [root, setRoot] = useState<CrewNodeDto | null>(null);
  const [openIds, setOpenIds] = useState<string[] | null>(null);
  // `worker` in the route opens on that worker (a deep link, for example from an action card).
  const [selected, setSelected] = useState(route.params.worker ? `agent:${route.params.worker}` : "");
  const [err, setErr] = useState("");
  const current = useRef<Target | null | undefined>(target);
  current.current = target;

  // The screen stays mounted while the hash changes: follow the route's run, and drop what belonged to the old one.
  const routeKey = routed ? `${routed.slug}\n${routed.runId}` : "";
  const seen = useRef(routeKey);
  useEffect(() => {
    if (seen.current === routeKey) return;
    seen.current = routeKey;
    setTarget(routed);
    setRoot(null);
    setOpenIds(null);
    setSelected("");
    setErr("");
    setGrewAt(null);
    openedAt.current = Date.now();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routeKey]);

  // When anything last happened: the campaign's own last write, or a worker's stream growing while this view watched.
  const [grewAt, setGrewAt] = useState<number | null>(null);
  const openedAt = useRef(Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), CLOCK_MS);
    return () => clearInterval(t);
  }, []);
  const onActivity = useCallback((at: number) => { setGrewAt(at); setNow(at); }, []);

  // `focus=<node id>` (from the Run view's task graph) selects that node (a task's latest worker), and opens its ancestors, once the tree has loaded.
  const focus = route.params.focus ?? "";
  const focused = useRef("");
  useEffect(() => {
    if (!root || !focus || focused.current === `${routeKey}\n${focus}`) return;
    // A task group (`task:<tid>`) has no stream or controls: open its latest round's worker (children are in round order).
    const node = find(root, focus);
    const target = node && focus.startsWith("task:") && node.children.length ? node.children[node.children.length - 1].id : focus;
    const path = pathTo(root, target);
    if (!path) return;
    focused.current = `${routeKey}\n${focus}`;
    setOpenIds((o) => Array.from(new Set([...(o ?? []), ...path.slice(0, -1)])));
    setSelected(target);
  }, [root, focus, routeKey]);

  // Which run: the route's, else the first active one.
  useEffect(() => {
    if (target !== undefined) return;
    let live = true;
    api.activeRuns().then(
      (a) => {
        if (!live) return;
        setErr("");
        const r = (a.runs as { run_id?: string; campaign?: string }[])[0];
        setTarget(r?.run_id && r.campaign ? { slug: r.campaign, runId: r.run_id } : null);
      },
      (e) => live && (setErr(problemDetail(e)), setTarget(null)),
    );
    return () => { live = false; };
  }, [target]);

  const load = useCallback(() => {
    if (!target) return;
    const asked = target;
    api.crew(target.slug, target.runId).then(
      (n) => { if (current.current !== asked) return; setRoot(n); setErr(""); setOpenIds((o) => o ?? withChildren(n)); setSelected((s) => s || initialPick(n)); },
      (e) => { if (current.current === asked) setErr(problemDetail(e)); },
    );
  }, [target]);

  useEffect(load, [load]);
  // Polled only while this run is live and the window shows; a finished run changes only with an event.
  usePoll(load, POLL_MS, !!target && liveRuns.includes(target.runId));
  // Leading and trailing: an event inside the gap is not lost, it is read when the gap ends. Hidden: one read on showing.
  useEffect(() => (target ? onChangedPaced(EVENT_MIN_GAP_MS, load) : undefined), [target, load]);

  // ⌘K: go to any worker of this run by name.
  useCommands(root ? agents(root).map((n) => ({ id: n.id, label: S.palette.goWorker(n.title), hint: statusLabel(n), run: () => setSelected(n.id) })) : []);

  const head = (title?: string) => (
    <header className="v-crew__head" data-tauri-drag-region>
      <Text as="h1" variant="label" className={title ? "v-crew__title" : undefined}>{S.crew.title}</Text>
      {title && <span className="v-crew__run">{title}</span>}
      {target && <span className="v-crew__id">{target.runId}</span>}
    </header>
  );
  // A run on another provider names it, and what it cannot do there (FG-09 G6); keyed so a new run never shows the old one's.
  const provider = target ? <RunProvider key={target.slug} slug={target.slug} /> : null;
  // Nothing to show yet: no run, a crew that could not be read, or one on its way. Each says so in words.
  if (!root) {
    // The problem stays up while it asks again; an answer clears it, so a retry that fails the same way still shows.
    const retry = () => { if (target) load(); else setTarget(undefined); };
    return (
      <div className="v-screen v-crew">
        {head()}
        {provider}
        <div className="v-screen__body v-crew__state">
          {err ? <Problem title={S.crew.failed} detail={err} retry={retry} />
            : target === null ? (
              <Empty title={S.crew.noRun} action={<>
                <Button variant="primary" onClick={() => go("home")}>{S.states.goHome}</Button>
                <Button variant="ghost" onClick={() => go("history")}>{S.states.openHistory}</Button>
              </>}>{S.crew.noRunHint}</Empty>
            ) : <div className="v-crew__tree v-crew__tree--loading"><LoadingRows rows={5} label={S.states.loading} /></div>}
        </div>
      </div>
    );
  }
  const picked = root && selected ? find(root, selected) : null;
  const run = root?.children.find((c) => c.kind === "run");

  // Has the run stalled? Something says it is working, but no process runs it, or it has been silent too long.
  // The core marks the run node in progress only while a process runs it, and "unknown" otherwise (crew in the desktop
  // core). That answer comes with this very tree, so it never races the projects store's own read of the live runs.
  const live = run ? run.status !== "unknown" : !!target && liveRuns.includes(target.runId);
  const updated = parseWhen(campaigns.find((c) => c.slug === target?.slug)?.updated ?? "");
  const lastMs = [Number.isFinite(updated) ? updated : null, grewAt].reduce<number | null>((a, b) => (b === null ? a : a === null ? b : Math.max(a, b)), null);
  const activeNow = agents(root).some((w) => (w.overlays as unknown[]).includes("activity"));
  const stall = stalledSince({ working: anyWorking(root), live, lastMs, watchedMs: now - openedAt.current, activeNow, now });
  const stalled = stall !== undefined;
  const banner = (n: CrewNodeDto) => stalled && anyWorking(n) && target ? <StallNote since={stall ?? null} live={live} slug={target.slug} runId={target.runId} /> : undefined;

  /** A worker's name as a person reads it: its role, else its task and round ("Backend · t2, round 1"). */
  const nameOf = (w: CrewNodeDto) => {
    if (w.role) return w.role;
    const tid = taskGroupOf(root, w.id);
    const task = tid ? find(root, `task:${tid}`) : null;
    return task ? `${task.title}, ${w.title}` : w.title;
  };
  const facts = (n: CrewNodeDto): Facts => ({
    ...statusShown(n, stalled), stalled: stalled && anyWorking(n) ? stall ?? null : undefined, live,
    slug: target?.slug ?? "", runId: target?.runId ?? "", lastMs, activeNow, nameOf, onActivity,
    taskOf: (w) => {
      const tid = taskGroupOf(root, w.id);
      return tid ? find(root, `task:${tid}`)?.title ?? tid : null;
    },
  });

  /** A worker's own pane: its live activity first. */
  const workerPane = (w: CrewNodeDto) => {
    const s = statusShown(w, stalled);
    if (isCore(w.id)) return <CoreWorkerPane key={w.id} node={w} status={s.label} shape={s.shape} banner={banner(w)} />;
    if (!target) return null;
    return <WorkerPane key={w.id} node={w} slug={target.slug} runId={target.runId} taskLocalId={taskGroupOf(root, w.id) ?? null} status={s.label} shape={s.shape} banner={banner(w)} onActivity={onActivity} />;
  };
  /** A task shows its latest round's worker; a task nothing has picked up says so in words. */
  const taskPane = (t: CrewNodeDto) => {
    const latest = t.children.filter((c) => isWorkerId(c.id)).pop();
    if (latest) return workerPane(latest);
    const s = statusShown(t, stalled);
    return (
      <section className="v-crew__pane v-crew__summary" aria-label={t.title}>
        <div className="v-crew__panehead">
          <Text as="h2" variant="headline-md">{t.title}</Text>
          <StatusDot status={s.shape} label={s.label} />
        </div>
        {banner(t)}
        <p className="v-crew__now">{stalled && anyWorking(t) ? S.crew.neverStarted : S.crew.waiting}</p>
      </section>
    );
  };
  const pane = !root || !picked ? null
    : isWorkerId(picked.id) ? workerPane(picked)
    : picked.kind === "task" || picked.id.startsWith("task:") ? taskPane(picked)
    : <RunSummary key={picked.id} scope={picked} f={facts(picked)} />;
  const pickedShown = picked ? statusShown(picked, stalled) : null;

  return (
    <div className="v-screen v-crew">
      {head(run?.title)}
      {provider}
      {err && <div role="alert" className="v-crew__err"><Text variant="body-sm">{S.crew.failed} {err}</Text></div>}
      <div className="v-crew__body">
        <div className="v-crew__tree">
          {root && openIds && (
            <CrewTree label={S.crew.treeLabel} nodes={[toNode(root, stalled)]} selected={selected} onSelect={setSelected} defaultOpen={openIds} />
          )}
        </div>
        {pane}
      </div>
      <ActionBar label={S.crew.barLabel}
        context={picked && pickedShown ? <span>{`${picked.role ?? picked.title}, ${pickedShown.label}`}</span> : undefined} />
    </div>
  );
}
