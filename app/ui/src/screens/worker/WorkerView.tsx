// One worker as its own tab (shell B): its stream, task, changes and controls, from the Crew screen's WorkerPane.
// The crew tree is read here only to find the worker's node and task; the project column already lists the crew.
import { useCallback, useEffect, useState } from "react";
import { Button } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, onChanged } from "../../ipc";
import type { CrewNodeDto } from "../../ipc";
import { useRoute } from "../../router";
import { Empty, LoadingRows, Problem, problemDetail } from "../states";
import { WorkerPane } from "../crew/WorkerPane";

const POLL_MS = 2000;

/** The worker's node and the run-local id of the task it is under. */
function locate(root: CrewNodeDto, agent: string): { node: CrewNodeDto; task: string | null } | null {
  const walk = (n: CrewNodeDto, task: string | null): { node: CrewNodeDto; task: string | null } | null => {
    if (n.id === `agent:${agent}`) return { node: n, task };
    for (const c of n.children) {
      const f = walk(c, n.id.startsWith("task:") ? n.id.slice("task:".length) : task);
      if (f) return f;
    }
    return null;
  };
  return walk(root, null);
}

export function WorkerView() {
  const [route, go] = useRoute();
  const { slug = "", id = "", worker = "" } = route.params;
  const [root, setRoot] = useState<CrewNodeDto | null>(null);
  const [err, setErr] = useState("");
  const load = useCallback(() => {
    if (!slug || !id) return;
    api.crew(slug, id).then((n) => { setRoot(n); setErr(""); }, (e) => setErr(problemDetail(e)));
  }, [slug, id]);
  useEffect(() => { setRoot(null); setErr(""); }, [slug, id]);
  useEffect(load, [load]);
  useEffect(() => {
    if (!slug || !id) return;
    const poll = setInterval(() => { if (!document.hidden) load(); }, POLL_MS);
    const off = onChanged(load);
    return () => { clearInterval(poll); off(); };
  }, [slug, id, load]);

  const found = root ? locate(root, worker) : null;
  if (err && !root) return <div className="v-crew__state"><Problem title={S.crew.failed} detail={err} retry={load} /></div>;
  if (!root) return <div className="v-crew__state"><LoadingRows rows={5} label={S.states.loading} /></div>;
  if (!found) {
    return (
      <div className="v-crew__state">
        <Empty title={S.crew.noRun} action={<Button onClick={() => go("orch")}>{S.views.orch}</Button>}>{S.crew.noRunHint}</Empty>
      </div>
    );
  }
  const status = found.node.status;
  const label = (S.status as Record<string, string>)[status] ?? S.crew.unknownSignal;
  return (
    <div className="v-screen v-worker">
      <WorkerPane key={found.node.id} node={found.node} slug={slug} runId={id} taskLocalId={found.task} status={label} />
    </div>
  );
}
