import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Section, Text } from "../../../components/ui/base";
import { ActionBar, CrewTree } from "../../../components/ui/desktop";
import type { CrewNode } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import type { StatusKey } from "../../../strings";
import { api, onChanged } from "../../ipc";
import { Empty, LoadingRows, Problem, problemDetail } from "../states";
import type { CrewNodeDto } from "../../ipc";
import { useCommands } from "../../palette";
import { useRoute } from "../../router";
import { WorkerPane } from "./WorkerPane";

/** A status or event is re-read at least this often: a tasks.json edit raises no event (AC-2.1.2: within 2 s). */
const POLL_MS = 2000;
/** Changed events arrive in bursts; one re-read per window is enough. */
const EVENT_MIN_GAP_MS = 500;

type Target = { slug: string; runId: string };
const STATUSES = new Set<string>(["pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed", "unknown"]);
const statusOf = (s: string): StatusKey => (STATUSES.has(s) ? (s as StatusKey) : "unknown");

/** The text next to the dot: the status, or "no signal" for a node nothing reports on, plus any overlay. */
function statusLabel(n: CrewNodeDto): string {
  const key = statusOf(n.status);
  const base = key === "unknown" ? S.crew.unknownSignal : S.status[key];
  const extra = (n.overlays as unknown[]).filter((o): o is string => typeof o === "string").map((o) => S.crew.overlay[o] ?? o);
  return [base, ...extra].join(", ");
}

function toNode(n: CrewNodeDto): CrewNode {
  return { id: n.id, label: n.title, status: statusOf(n.status), statusLabel: statusLabel(n), children: n.children.map(toNode) };
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
/** Every worker in the tree, in tree order. */
const agents = (n: CrewNodeDto, out: CrewNodeDto[] = []): CrewNodeDto[] => {
  if (n.id.startsWith("agent:")) out.push(n);
  n.children.forEach((c) => agents(c, out));
  return out;
};
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
  const routed = route.params.slug && route.params.id ? { slug: route.params.slug, runId: route.params.id } : undefined;
  const [target, setTarget] = useState<Target | null | undefined>(routed);
  const [root, setRoot] = useState<CrewNodeDto | null>(null);
  const [openIds, setOpenIds] = useState<string[] | null>(null);
  // `worker` in the route opens on that worker (a deep link, for example from an action card).
  const [selected, setSelected] = useState(route.params.worker ? `agent:${route.params.worker}` : "");
  const [err, setErr] = useState("");
  const last = useRef(0);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routeKey]);

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
    last.current = Date.now();
    const asked = target;
    api.crew(target.slug, target.runId).then(
      (n) => { if (current.current !== asked) return; setRoot(n); setErr(""); setOpenIds((o) => o ?? withChildren(n)); setSelected((s) => s || n.id); },
      (e) => { if (current.current === asked) setErr(problemDetail(e)); },
    );
  }, [target]);

  useEffect(load, [load]);
  useEffect(() => {
    if (!target) return;
    const poll = setInterval(() => { if (!document.hidden) load(); }, POLL_MS);
    // Leading and trailing: an event inside the gap is not lost, it is read when the gap ends.
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged(() => {
      const wait = EVENT_MIN_GAP_MS - (Date.now() - last.current);
      if (wait <= 0) load();
      else if (later === undefined) later = setTimeout(() => { later = undefined; load(); }, wait);
    });
    return () => { clearInterval(poll); clearTimeout(later); off(); };
  }, [target, load]);

  // ⌘K: go to any worker of this run by name.
  useCommands(root ? agents(root).map((n) => ({ id: n.id, label: S.palette.goWorker(n.title), hint: statusLabel(n), run: () => setSelected(n.id) })) : []);

  const head = (title?: string) => (
    <header className="v-crew__head">
      <Text as="h1" variant="label" className={title ? "v-crew__title" : undefined}>{S.crew.title}</Text>
      {title && <span className="v-crew__run">{title}</span>}
      {target && <span className="v-crew__id">{target.runId}</span>}
    </header>
  );
  // Nothing to show yet: no run, a crew that could not be read, or one on its way. Each says so in words.
  if (!root) {
    // The problem stays up while it asks again; an answer clears it, so a retry that fails the same way still shows.
    const retry = () => { if (target) load(); else setTarget(undefined); };
    return (
      <div className="v-screen v-crew">
        {head()}
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
  return (
    <div className="v-screen v-crew">
      {head(run?.title)}
      {err && <div role="alert" className="v-crew__err"><Text variant="body-sm">{S.crew.failed} {err}</Text></div>}
      <div className="v-crew__body">
        <div className="v-crew__tree">
          {root && openIds && (
            <CrewTree label={S.crew.treeLabel} nodes={[toNode(root)]} selected={selected} onSelect={setSelected} defaultOpen={openIds} />
          )}
        </div>
        {root && picked && (picked.id.startsWith("agent:") ? (
          target && <WorkerPane key={picked.id} node={picked} slug={target.slug} runId={target.runId} taskLocalId={taskGroupOf(root, picked.id) ?? null} status={statusLabel(picked)} />
        ) : (
          <div className="v-crew__pane v-crew__pane--facts">
            <Section title={S.crew.details}>
              <dl className="v-crew__facts" aria-label={S.crew.details}>
                <dt>{S.crew.status}</dt><dd>{statusLabel(picked)}</dd>
                <dt>{S.crew.kind}</dt><dd>{picked.kind}</dd>
                {picked.engine_id && (<><dt>{S.crew.engineId}</dt><dd>{picked.engine_id}</dd></>)}
              </dl>
            </Section>
          </div>
        ))}
      </div>
      <ActionBar label={S.crew.barLabel}
        context={picked ? <span>{`${picked.role ?? picked.title}, ${statusLabel(picked)}`}</span> : undefined} />
    </div>
  );
}
