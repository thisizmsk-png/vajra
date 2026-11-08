// The inspector (shell B, ⌘I): facts about what the main area shows. It is mounted only while open, so it reads
// nothing while closed (it starts closed below 1440 px). Everything in it comes from the core; no stand-in numbers.
import { useEffect, useState } from "react";
import { StatusDot } from "../../components/ui/base";
import { S } from "../../strings";
import type { StatusKey } from "../../strings";
import { api, onChanged } from "../ipc";
import type { Campaign, EvidenceRow, WorktreeInfo } from "../ipc";
import type { ActiveCrew, Project, WorkerRow } from "../projects/contract";
import { sessionStatus } from "../projects/adapter";

const SHOWN = 8;
const EV: Record<string, StatusKey> = { pass: "pass", fail: "fail", not_run: "unknown", unverified: "unknown", checking: "ready", stale: "blocked" };
const DOT = { running: "in_progress", done: "done", stopped: "blocked", failed: "failed", unknown: "unknown" } as const;

function Criteria({ slug }: { slug: string }) {
  const [rows, setRows] = useState<EvidenceRow[] | null>(null);
  const [err, setErr] = useState(false);
  useEffect(() => {
    let live = true;
    const load = () => api.evidenceList(slug).then((r) => { if (live) { setRows(Array.isArray(r?.rows) ? r.rows : []); setErr(false); } }, () => live && setErr(true));
    setRows(null);
    void load();
    const off = onChanged(() => void load());
    return () => { live = false; off(); };
  }, [slug]);
  return (
    <section className="v-insp__sec" aria-labelledby="v-insp-criteria">
      <h3 id="v-insp-criteria" className="v-insp__h">{S.inspect.criteria}</h3>
      {err && <p className="v-insp__none">{S.inspect.criteriaFailed}</p>}
      {rows && rows.length === 0 && <p className="v-insp__none">{S.inspect.noCriteria}</p>}
      {rows && rows.length > 0 && (
        <ul className="v-insp__list">
          {rows.slice(0, SHOWN).map((r) => (
            <li key={`${r.task}/${r.criterion}`}><StatusDot status={EV[r.status] ?? "unknown"} className="v-dot-sr" /><span className="v-insp__crit">{r.criterion}</span><span className="v-insp__muted">{r.title ?? r.task}</span></li>
          ))}
          {rows.length > SHOWN && <li className="v-insp__muted">{S.inspect.more(rows.length - SHOWN)}</li>}
        </ul>
      )}
    </section>
  );
}

function Worktree({ worker }: { worker: string }) {
  const [info, setInfo] = useState<WorktreeInfo | null | undefined>(undefined);
  useEffect(() => {
    let live = true;
    setInfo(undefined);
    api.worktreeInfo(worker).then((i) => live && setInfo(i), () => live && setInfo(null));
    return () => { live = false; };
  }, [worker]);
  if (info === undefined) return null;
  return (
    <dl className="v-kv">
      <dt>{S.inspect.branch}</dt><dd>{info?.branch ?? S.inspect.noWorktree}</dd>
      {info && <><dt>{S.inspect.worktree}</dt><dd title={info.worktree}>{info.worktree}</dd></>}
    </dl>
  );
}

export function Inspector({ project, focus, crew, worker }: { project: Project; focus: Campaign | null; crew: ActiveCrew | null; worker: WorkerRow | null }) {
  const counts = new Map<StatusKey, number>();
  for (const w of crew?.workers ?? []) counts.set(w.status, (counts.get(w.status) ?? 0) + 1);
  return (
    <aside className="v-insp" aria-label={S.frame.inspector}>
      {worker ? (
        <section className="v-insp__sec" aria-labelledby="v-insp-worker">
          <h3 id="v-insp-worker" className="v-insp__h">{S.inspect.worker}</h3>
          <div className="v-insp__title">{worker.name}</div>
          <dl className="v-kv">
            <dt>{S.inspect.status}</dt><dd><StatusDot status={worker.status} label={worker.statusLabel} /></dd>
            <dt>{S.inspect.task}</dt><dd title={worker.task}>{worker.task}</dd>
          </dl>
          <Worktree worker={worker.id} />
        </section>
      ) : (
        <section className="v-insp__sec" aria-labelledby="v-insp-campaign">
          <h3 id="v-insp-campaign" className="v-insp__h">{S.inspect.campaign}</h3>
          {focus ? (
            <>
              <div className="v-insp__title">{focus.goal ?? focus.slug}</div>
              <div className="v-insp__muted">{focus.slug}</div>
              <dl className="v-kv">
                <dt>{S.inspect.status}</dt><dd><StatusDot status={DOT[sessionStatus(focus)]} label={S.history.state[focus.status ?? ""] ?? S.status.unknown} /></dd>
                {typeof focus.total === "number" && <><dt>{S.inspect.criteria}</dt><dd className="v-num">{S.column.progress(focus.passing ?? 0, focus.total)}</dd></>}
                <dt>{S.inspect.folder}</dt><dd title={project.root ?? undefined}>{project.root ?? S.projects.unsorted}</dd>
              </dl>
            </>
          ) : <p className="v-insp__none">{S.inspect.none}</p>}
        </section>
      )}
      {!worker && focus && <Criteria slug={focus.slug} />}
      {!worker && counts.size > 0 && (
        <section className="v-insp__sec" aria-labelledby="v-insp-workers">
          <h3 id="v-insp-workers" className="v-insp__h">{S.inspect.workers}</h3>
          <ul className="v-insp__list">
            {[...counts].map(([k, n]) => <li key={k}><StatusDot status={k} /><span className="v-num v-insp__end">{n}</span></li>)}
          </ul>
        </section>
      )}
    </aside>
  );
}
