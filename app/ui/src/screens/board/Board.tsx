// The project's board (FG-06): one campaign's tasks in Backlog, Running, Review and Done, with a needs-you badge.
// Read-only (ADR-15 D7): a card moves when its work and its checks move it, never by hand. Opening a card goes to its
// worker in the crew (a task) or to the campaign's Review (a plan or a question).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { Badge, Button, StatusDot } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, onChanged } from "../../ipc";
import type { RunOverview } from "../../ipc";
import { boardCards } from "../../projects/adapter";
import type { BoardCard, Lane } from "../../projects/contract";
import { useCurrentProject } from "../../projects/current";
import { useProjectState } from "../../projects/state";
import { useProjects } from "../../projects/store";
import { useCommands } from "../../palette";
import { useRoute } from "../../router";
import { Empty, LoadingRows, Problem, problemDetail } from "../states";
import { ProviderLine } from "../ProviderLine";

const LANES: Lane[] = ["backlog", "running", "review", "done"];
const EVENT_MIN_GAP_MS = 500;

export function Board() {
  const { focus, project } = useCurrentProject();
  const { rev, failed: noRevision } = useProjectState(project?.id ?? null);
  const P = useProjects();
  const [, go] = useRoute();
  const [rows, setRows] = useState<Parameters<typeof boardCards>[1] | null>(null);
  const [run, setRun] = useState<RunOverview | null>(null);
  const [err, setErr] = useState("");
  const [onlyYou, setOnlyYou] = useState(false);
  // Every load takes a number; only the newest may set state. A slug check is not enough: two loads of the same
  // campaign overlap when events arrive while one is out, and the older answer must not land last.
  const seq = useRef(0);
  const last = useRef(0);
  const seenRev = useRef<number | null>(null);
  const [active, setActive] = useState<string | null>(null);
  const refs = useRef(new Map<string, HTMLButtonElement>());

  const load = useCallback(() => {
    if (!focus) return;
    const mine = ++seq.current;
    last.current = Date.now();
    const slug = focus;
    void Promise.allSettled([api.runOverview(slug), api.evidenceList(slug)]).then(([o, e]) => {
      if (mine !== seq.current) return;
      if (o.status === "rejected") { setErr(problemDetail(o.reason)); return; }
      setRun(o.value);
      setRows(e.status === "fulfilled" && Array.isArray(e.value?.rows) ? e.value.rows : []);
      setErr("");
    });
  }, [focus]);
  // The inbox is the store's copy: the store reads it on every change, so a second scan here bought nothing. Until the
  // store's first read lands the board waits, so the needs-you cards never appear late. A failed read is an empty list.
  const cards = useMemo<BoardCard[] | null>(
    () => (run && rows && P.state !== "loading" ? boardCards(run, rows, P.inbox) : null),
    [run, rows, P.state, P.inbox],
  );

  // A new campaign, or leaving the board, retires whatever is still out.
  useEffect(() => { setRows(null); setRun(null); setErr(""); return () => { seq.current++; }; }, [focus]);
  useEffect(load, [load]);
  // The project's revision is the one authority: heavy reads happen only when it moves (ADR-15 §4). Without a
  // revision (an older core) the raw change events drive the reload, as before.
  useEffect(() => { seenRev.current = null; }, [project?.id]);
  useEffect(() => {
    if (rev === null) return;
    if (seenRev.current === null) { seenRev.current = rev; return; }
    if (rev !== seenRev.current) { seenRev.current = rev; load(); }
  }, [rev, load]);
  useEffect(() => {
    if (!focus || !noRevision) return;
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged(() => {
      const wait = EVENT_MIN_GAP_MS - (Date.now() - last.current);
      if (wait <= 0) load();
      else if (later === undefined) later = setTimeout(() => { later = undefined; load(); }, wait);
    });
    return () => { off(); clearTimeout(later); };
  }, [focus, load, noRevision]);

  const open = (c: BoardCard) => {
    if (!focus) return;
    if (c.kind === "task" && run) go("crew", { slug: focus, id: run.run_id, focus: `task:${c.local}` });
    else go("review", { slug: focus });
  };
  const shown = useMemo(() => (cards ?? []).filter((c) => !onlyYou || c.needsYou), [cards, onlyYou]);
  const waiting = (cards ?? []).filter((c) => c.needsYou).length;
  const columns = LANES.map((l) => shown.filter((c) => c.lane === l));
  const stop = columns.flat().find((c) => c.key === active)?.key ?? columns.flat()[0]?.key;
  // D7: arrow keys move focus between cards, never the cards themselves.
  const onKey = (e: KeyboardEvent<HTMLElement>, c: BoardCard) => {
    const col = columns.findIndex((l) => l.some((x) => x.key === c.key));
    const at = columns[col].findIndex((x) => x.key === c.key);
    let next: BoardCard | undefined;
    if (e.key === "ArrowDown") next = columns[col][at + 1];
    else if (e.key === "ArrowUp") next = columns[col][at - 1];
    else if (e.key === "Home") next = columns[col][0];
    else if (e.key === "End") next = columns[col][columns[col].length - 1];
    else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      const step = e.key === "ArrowRight" ? 1 : -1;
      for (let i = col + step; i >= 0 && i < columns.length && !next; i += step) next = columns[i][Math.min(at, columns[i].length - 1)];
    } else return;
    e.preventDefault();
    if (!next) return;
    setActive(next.key);
    refs.current.get(next.key)?.focus();
  };
  useCommands(focus ? [{ id: "needs-you", label: S.board.needsYouFilter, run: () => setOnlyYou((x) => !x) }] : []);

  // With no project list there is no project to have a campaign: "none yet" would be a guess, so say what failed.
  if (!focus && P.state === "error") {
    return (
      <div className="v-board__state">
        <Problem title={S.frame.loadFailed} detail={P.error} retry={P.reload} />
      </div>
    );
  }
  if (!focus) {
    return (
      <div className="v-board__state">
        <Empty title={S.board.noCampaign}>{S.board.noCampaignHint}</Empty>
      </div>
    );
  }
  return (
    <section className="v-screen v-board" aria-label={`${S.board.title}, ${project?.name ?? ""}`}>
      <div className="v-board__bar">
        <span className="v-board__note">{S.board.readOnly}</span>
        {waiting > 0 && (
          <Button size="sm" variant="ghost" aria-pressed={onlyYou} onClick={() => setOnlyYou((x) => !x)} className={onlyYou ? "v-board__filter is-on" : "v-board__filter"}>
            {S.board.needsYouFilter}
          </Button>
        )}
      </div>
      <p role="status" className="v-sr">{waiting > 0 ? S.frame.needsYou(waiting) : ""}</p>
      {err && <div className="v-board__state"><Problem title={S.board.failed} detail={err} retry={load} /></div>}
      {!err && !cards && <div className="v-board__state"><LoadingRows rows={4} label={S.states.loading} /></div>}
      {!err && cards && (
        <div className="v-board__lanes">
          {LANES.map((lane, li) => {
            const list = columns[li];
            const name = S.board.lanes[lane];
            return (
              <section key={lane} className="v-lane" role="region" aria-label={S.board.of(name, list.length)}>
                <h2 className="v-lane__h">{name}<span className="v-num">{list.length}</span></h2>
                {list.length === 0 ? <p className="v-lane__none">{S.board.empty}</p> : (
                  <ul className="v-lane__list">
                    {list.map((c) => (
                      <li key={c.key}>
                        <button
                          type="button"
                          ref={(el) => { if (el) refs.current.set(c.key, el); else refs.current.delete(c.key); }}
                          tabIndex={c.key === stop ? 0 : -1}
                          className={c.needsYou ? "v-card v-card--you" : "v-card"}
                          onClick={() => open(c)}
                          onFocus={() => setActive(c.key)}
                          onKeyDown={(e) => onKey(e, c)}
                        >
                          <span className="v-card__top">
                            <span className="v-card__id">{c.kind === "task" ? c.local : c.kind === "plan" ? S.board.plan : S.board.question}</span>
                            {c.needsYou && <Badge status="warning" className="v-card__you">{S.board.needsYou}</Badge>}
                          </span>
                          <span className="v-card__t">{c.title}</span>
                          <span className="v-card__m">
                            {c.kind === "task" && <StatusDot status={c.status} />}
                            {c.criteria && <span className="v-num">{S.board.criteria(c.criteria.pass, c.criteria.total)}</span>}
                            {c.owner && <span className="v-card__owner">{c.owner}</span>}
                          </span>
                          {c.provider && <ProviderLine p={c.provider} />}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            );
          })}
        </div>
      )}
    </section>
  );
}
