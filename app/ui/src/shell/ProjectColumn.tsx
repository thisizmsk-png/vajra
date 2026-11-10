// The project column (shell B): the project's folder, its Director-General, its workers, and its own sessions by day.
// Only this project's work is listed here (FG-03): other projects are one tab away, never mixed in.
import { useMemo, useState } from "react";
import { Button, Icon, StatusDot } from "../../components/ui/base";
import { S } from "../../strings";
import { sessionsByDay } from "../projects/adapter";
import type { ActiveCrew, Project, WorkerRow } from "../projects/contract";
import { api, newActionId } from "../ipc";
import type { Campaign, CloseFinishedResult } from "../ipc";
import { unsavedText, useCloseout } from "../projects/closeout";
import type { Route } from "../router";
import { ProviderLine } from "../screens/ProviderLine";

export function ProjectColumn({ project, campaigns, crew, route, onOrch, onWorker, onSession }: {
  project: Project;
  campaigns: Campaign[];
  /** The live run, only when it is this project's. */
  crew: ActiveCrew | null;
  route: Route;
  onOrch: () => void;
  onWorker: (w: WorkerRow) => void;
  onSession: (slug: string) => void;
}) {
  const [q, setQ] = useState("");
  const days = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return sessionsByDay(needle ? campaigns.filter((c) => `${c.goal ?? ""} ${c.slug}`.toLowerCase().includes(needle)) : campaigns);
  }, [campaigns, q]);
  const focus = crew ? campaigns.find((c) => c.slug === crew.campaign) : campaigns[0];
  const live = crew?.workers.filter((w) => w.status === "in_progress" || w.status === "claimed" || w.status === "question").length ?? 0;
  const onOrchView = route.screen === "orch";
  const closeout = useCloseout(crew?.campaign ?? null, crew?.runId ?? null);
  const [closing, setClosing] = useState(false);
  const [closed, setClosed] = useState<CloseFinishedResult | "failed" | null>(null);
  // Enabled while some finished worker is still open; the core decides what is safe, this only offers the click.
  const anyFinished = [...closeout.rows.values()].some((r) => r.state !== "closed" && r.state !== "working" && r.state !== "waiting_on_you");
  const closeFinished = () => {
    if (closing) return;
    setClosing(true);
    void api.actCloseFinished({ action_id: newActionId(), project_id: project.id, expect_generation: null }).then(
      (r) => setClosed(r.result ?? "failed"),
      () => setClosed("failed"),
    ).finally(() => { setClosing(false); closeout.reload(); });
  };
  const nameOf = (id: string, fallback: string) => crew?.workers.find((w) => w.id === id)?.name ?? (fallback || id);

  return (
    <aside className="v-pcol" aria-label={S.column.label(project.name)}>
      <div className="v-pcol__meta" title={project.root ?? undefined}>
        <span className="v-pcol__path">{project.root ?? S.projects.unsortedHint}</span>
      </div>
      {!project.bucket && (
        <button type="button" className={onOrchView ? "v-dg is-active" : "v-dg"} onClick={onOrch} aria-current={onOrchView ? "page" : undefined}>
          <span className="v-dg__t">
            <Icon name="director" />
            <span>{S.views.orch}</span>
            <span className="v-dg__run">{crew ? S.column.leading(crew.runId) : S.column.noRun}</span>
          </span>
          <span className="v-dg__c">{focus?.goal ?? (crew ? crew.campaign : S.column.waiting)}</span>
          {focus && typeof focus.total === "number" && focus.total > 0 && (
            <span className="v-dg__m">
              <span className="v-meter" role="img" aria-label={S.column.progressLabel(focus.passing ?? 0, focus.total)}>
                <i style={{ width: `${Math.min(100, ((focus.passing ?? 0) / focus.total) * 100)}%` }} />
              </span>
              <span className="v-num">{S.column.progress(focus.passing ?? 0, focus.total)}</span>
            </span>
          )}
        </button>
      )}
      <div className="v-pcol__scroll">
        {!project.bucket && (
          <section aria-labelledby="v-pcol-workers">
            <h2 id="v-pcol-workers" className="v-pcol__h">{S.column.workers}
              {crew && crew.workers.length > 0 && <span className="v-pcol__count">{S.column.workersActive(live, crew.workers.length)}</span>}
              {crew && crew.workers.length > 0 && (
                <Button size="sm" variant="ghost" className="v-pcol__bulk" disabled={closing || !anyFinished} busy={closing} onClick={closeFinished} title={S.closeout.closeFinishedHint}>{S.closeout.closeFinished}</Button>
              )}
            </h2>
            <div role="status" aria-live="polite">
              {closed && (
                <div className="v-closeout">
                  {closed === "failed" ? <p>{S.closeout.notDone}</p> : <>
                    <p>{S.closeout.closedCount(closed.closed.length)}</p>
                    {closed.skipped.length > 0 && (<>
                      <p>{S.closeout.keptCount(closed.skipped.length)}</p>
                      <ul>{closed.skipped.map((k) => (
                        <li key={k.worker}>{S.closeout.kept(nameOf(k.worker, k.label), `${S.closeout.reason[k.reason] ?? k.reason}${k.reason === "unsaved_work" || k.reason === "terminal_busy" ? ` (${unsavedText(k)})` : ""}`)}</li>
                      ))}</ul>
                    </>)}
                  </>}
                  <Button size="sm" variant="ghost" onClick={() => setClosed(null)}>{S.closeout.dismiss}</Button>
                </div>
              )}
            </div>
            {crew && crew.workers.length > 0 ? crew.workers.map((w) => {
              const active = route.screen === "worker" && route.params.worker === w.id;
              const co = closeout.rows.get(w.id);
              const isClosed = co?.state === "closed";
              return (
                <button key={w.id} type="button" className={`${active ? "v-w2 is-active" : "v-w2"}${isClosed ? " is-closed" : ""}`} onClick={() => onWorker(w)} aria-current={active ? "page" : undefined}>
                  <span className="v-w2__a"><span className="v-w2__name">{w.name}</span>
                    {isClosed && <span className="v-w2__tag" title={S.closeout.closedHint}>{S.closeout.closed}</span>}
                    {(co?.state === "unsaved_work" || co?.state === "terminal_busy") && <span className="v-w2__tag v-w2__tag--warn" title={unsavedText(co)}>{co.state === "unsaved_work" ? S.closeout.unsaved : S.closeout.busyTag}</span>}
                    <StatusDot status={w.status} label={w.statusLabel} className="v-w2__s" /></span>
                  <span className="v-w2__t">{w.task}</span>
                  {w.provider && <ProviderLine p={w.provider} compact />}
                </button>
              );
            }) : <p className="v-pcol__none">{crew ? S.column.noWorkers : S.column.workersHint}</p>}
          </section>
        )}
        <section aria-labelledby="v-pcol-sessions">
          <h2 id="v-pcol-sessions" className="v-pcol__h">{S.column.sessions}{campaigns.length > 0 && <span className="v-pcol__count">{campaigns.length}</span>}</h2>
          {campaigns.length > 4 && (
            <label className="v-pcol__q">
              <Icon name="search" size={12} />
              <input type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder={S.column.searchSessions(project.name)} aria-label={S.column.searchSessions(project.name)} />
            </label>
          )}
          {campaigns.length === 0 && <p className="v-pcol__none">{S.column.noSessions}</p>}
          {campaigns.length > 0 && days.length === 0 && <p className="v-pcol__none">{S.column.noMatch}</p>}
          {days.map((d) => (
            <div key={d.key} role="group" aria-label={`${d.label}, ${d.summary}`}>
              <div className="v-day" aria-hidden="true"><span>{d.label}</span><em>{d.summary}</em></div>
              {d.items.map((s) => {
                const active = route.screen === "review" && route.params.slug === s.slug;
                return (
                  <button key={s.slug} type="button" className={active ? "v-srow is-active" : "v-srow"} onClick={() => onSession(s.slug)} aria-current={active ? "page" : undefined} title={s.title}>
                    <StatusDot status={s.dot} className="v-dot-sr" />
                    <span className="v-srow__t">{s.title}</span>
                    <span className="v-srow__time">{s.time}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </section>
      </div>
    </aside>
  );
}
