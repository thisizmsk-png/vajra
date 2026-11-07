import { useCallback, useEffect, useRef, useState } from "react";
import { Section, Text } from "../../../components/ui/base";
import { CrewTree } from "../../../components/ui/desktop";
import type { CrewNode } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import type { StatusKey } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { CrewNodeDto } from "../../ipc";
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
  return [base, ...extra].join(" · ");
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
  const [route] = useRoute();
  const routed = route.params.slug && route.params.id ? { slug: route.params.slug, runId: route.params.id } : undefined;
  const [target, setTarget] = useState<Target | null | undefined>(routed);
  const [root, setRoot] = useState<CrewNodeDto | null>(null);
  const [openIds, setOpenIds] = useState<string[] | null>(null);
  const [selected, setSelected] = useState("");
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

  // `focus=<node id>` (from the Run view's task graph) selects that node, and opens its ancestors, once the tree has loaded.
  const focus = route.params.focus ?? "";
  const focused = useRef("");
  useEffect(() => {
    if (!root || !focus || focused.current === `${routeKey}\n${focus}`) return;
    const path = pathTo(root, focus);
    if (!path) return;
    focused.current = `${routeKey}\n${focus}`;
    setOpenIds((o) => Array.from(new Set([...(o ?? []), ...path.slice(0, -1)])));
    setSelected(focus);
  }, [root, focus, routeKey]);

  // Which run: the route's, else the first active one.
  useEffect(() => {
    if (target !== undefined) return;
    let live = true;
    api.activeRuns().then(
      (a) => {
        if (!live) return;
        const r = (a.runs as { run_id?: string; campaign?: string }[])[0];
        setTarget(r?.run_id && r.campaign ? { slug: r.campaign, runId: r.run_id } : null);
      },
      (e) => live && (setErr(errorText(e)), setTarget(null)),
    );
    return () => { live = false; };
  }, [target]);

  const load = useCallback(() => {
    if (!target) return;
    last.current = Date.now();
    const asked = target;
    api.crew(target.slug, target.runId).then(
      (n) => { if (current.current !== asked) return; setRoot(n); setErr(""); setOpenIds((o) => o ?? withChildren(n)); setSelected((s) => s || n.id); },
      (e) => { if (current.current === asked) setErr(errorText(e)); },
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

  if (target === null && !err) {
    return (
      <div className="v-crew">
        <Text as="h1" variant="headline-lg">{S.crew.title}</Text>
        <Text variant="body">{S.crew.noRun}</Text>
        <Text variant="body-sm" muted>{S.crew.noRunHint}</Text>
      </div>
    );
  }
  const picked = root && selected ? find(root, selected) : null;
  return (
    <div className="v-crew">
      <Text as="h1" variant="headline-lg">{S.crew.title}</Text>
      {err && <div role="alert"><Text variant="body-sm">{S.crew.failed} {err}</Text></div>}
      {root && openIds && (
        <div className="v-crew__body">
          <CrewTree label={S.crew.treeLabel} nodes={[toNode(root)]} selected={selected} onSelect={setSelected} defaultOpen={openIds} />
          {picked && (
            <div className="v-crew__side">
            {picked.id.startsWith("agent:") ? (
              target && <WorkerPane key={picked.id} node={picked} slug={target.slug} runId={target.runId} taskLocalId={root ? taskGroupOf(root, picked.id) ?? null : null} status={statusLabel(picked)} />
            ) : (
              <Section title={S.crew.details}>
                <dl className="v-crew__facts" aria-label={S.crew.details}>
                  <dt>{S.crew.status}</dt><dd>{statusLabel(picked)}</dd>
                  <dt>{S.crew.kind}</dt><dd>{picked.kind}</dd>
                  {picked.engine_id && (<><dt>{S.crew.engineId}</dt><dd>{picked.engine_id}</dd></>)}
                </dl>
              </Section>
            )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
