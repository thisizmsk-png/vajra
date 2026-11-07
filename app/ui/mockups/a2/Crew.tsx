import { Act, ShellG } from "./Shell";
import { DiffCount, StatusDot } from "../shared";
import { agents, approval, backendCommands, backendFiles, backendStream, criteria, phases, run, type Agent } from "../data";
import { IcBoard, IcCheck, IcFile, IcList, IcMessage, IcRedirect, IcStop, IcTarget, IcTerminal } from "./icons";

/** Crew grouped by what it needs from you (AO's attention zones), not by role. */
const groups: Array<{ name: string; ids: string[] }> = [
  { name: "Needs you", ids: ["security", "backend-1"] },
  { name: "Working", ids: ["lead", "backend", "frontend"] },
  { name: "Waiting", ids: ["verifier"] },
  { name: "Done", ids: ["architect"] },
];
const byId = (id: string) => agents.find((a) => a.id === id) as Agent;
const asks: Record<string, { status: "question" | "needs_approval"; label: string }> = {
  "backend-1": { status: "question", label: "Asked a question" },
};

function split(path: string) {
  const i = path.lastIndexOf("/");
  return <><span className="dir">{path.slice(0, i + 1)}</span>{path.slice(i + 1)}</>;
}

export function CrewG({ inert = false }: { inert?: boolean }) {
  const sel = byId("backend");
  return (
    <ShellG
      current="run"
      bar={
        <>
          <div className="ctx"><b>{sel.role}</b><span className="muted">{sel.taskId} {sel.task}</span></div>
          <div className="acts">
            <Act label="Approve plan" keys={["⌘", "↵"]} primary />
            <span className="sep" />
            <Act label="Message" keys={["M"]} />
            <Act label="Redirect" keys={["R"]} />
            <Act label="Stop worker" keys={["⌘", "."]} />
            <span className="sep" />
            <Act label="All actions" keys={["⌘", "K"]} />
          </div>
        </>
      }
    >
      <div className="g-crewview" {...(inert ? ({ inert: "" } as Record<string, string>) : {})}>
        <header className="g-head">
          <div className="g-crumb"><span className="muted">Runs</span><span className="muted">/</span>{run.title}</div>
          <div className="g-phases" aria-label="Phases">
            {phases.map((p) => (
              <span key={p.name} className={`g-ph g-ph--${p.state}`} aria-current={p.state === "live" ? "step" : undefined}>
                {p.state === "done" ? <IcCheck /> : null}{p.name}
              </span>
            ))}
          </div>
          <div className="g-facts">
            <span>Round <b>{run.round}</b> of {run.maxRounds}</span>
            <span><b>{run.elapsed}</b></span>
            <span><DiffCount added={331} removed={23} /></span>
          </div>
          <div className="g-seg" role="radiogroup" aria-label="Crew view">
            <button role="radio" aria-checked="true"><IcList />List</button>
            <button role="radio" aria-checked="false"><IcBoard />Board</button>
          </div>
        </header>

        <div className="g-gate" role="region" aria-label="Waiting for you">
          <StatusDot status="needs_approval" label="Plan change" />
          <div className="txt">
            <div><b>{approval.title}</b> <span className="muted">from {approval.from} at {approval.at}</span></div>
            <div className="chg">
              <span className="c"><span className="p">+</span>t6 Security review</span>
              <span className="c"><span className="t">~</span>t5 also waits on t6</span>
              <span className="more">{approval.cost}</span>
            </div>
          </div>
          <div className="acts">
            <button className="g-btn g-btn--ghost">Open in inbox</button>
            <button className="g-btn">Reject</button>
            <button className="g-btn g-btn--primary">Approve plan</button>
          </div>
        </div>

        <div className="g-body">
          <div className="g-crew" role="tree" aria-label="Crew">
            {groups.map((g) => (
              <div key={g.name} role="group" aria-label={g.name}>
                <div className="g-grp"><span>{g.name}</span><span className="num">{g.ids.length}</span></div>
                {g.ids.map((id) => {
                  const a = byId(id);
                  const ask = asks[id];
                  return (
                    <div key={id} role="treeitem" aria-selected={id === sel.id} tabIndex={id === sel.id ? 0 : -1} className="g-ag g-dot">
                      <StatusDot status={ask?.status ?? a.status} label={ask?.label ?? a.statusLabel} />
                      <span className="n">{a.role}{a.taskId ? <span className="tid">{a.taskId}</span> : null}</span>
                      <span className="r">{a.added !== undefined ? <DiffCount added={a.added} removed={a.removed} /> : a.lastAt}</span>
                      <span className="task">{ask ? ask.label : a.status === "in_progress" ? a.last : a.statusLabel ?? a.task}</span>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>

          <section className="g-pane" aria-label={`${sel.role} worker`}>
            <div className="g-panehead">
              <span className="who">{sel.role}</span>
              <span className="task">{sel.taskId} {sel.task}</span>
              <div className="acts">
                <button className="g-btn g-btn--ghost g-btn--sm" aria-label="Message Backend"><IcMessage /></button>
                <button className="g-btn g-btn--ghost g-btn--sm" aria-label="Redirect Backend"><IcRedirect /></button>
                <button className="g-btn g-btn--ghost g-btn--sm" aria-label="Stop Backend"><IcStop /></button>
              </div>
            </div>
            <div className="g-tabs" role="tablist" aria-label="Worker views">
              <button role="tab" aria-selected="true" className="g-tab">Stream</button>
              <button role="tab" aria-selected="false" className="g-tab">Diff <span className="c">{sel.files}</span></button>
              <button role="tab" aria-selected="false" className="g-tab">Task</button>
              <button role="tab" aria-selected="false" className="g-tab">Worktree</button>
            </div>
            <div className="g-tools" role="radiogroup" aria-label="Show in stream">
              <button role="radio" aria-checked="true" className="g-chip">All <span className="c">11</span></button>
              <button role="radio" aria-checked="false" className="g-chip">Edits <span className="c">5</span></button>
              <button role="radio" aria-checked="false" className="g-chip">Commands <span className="c">2</span></button>
              <button role="radio" aria-checked="false" className="g-chip">Messages <span className="c">1</span></button>
              <span className="g-follow"><span className="sw" role="switch" aria-checked="true" aria-label="Follow the live line" tabIndex={0} />Follow</span>
            </div>
            <div className="g-stream" role="log" aria-label="Backend stream, read only">
              {backendStream.map((e, i) =>
                e.kind === "action" ? (
                  <div key={i} className="g-card" role="article">
                    <div className="h"><b>{e.from} sent a message</b><span>{e.at}</span><span className="st">{e.state}</span></div>
                    {e.text}
                  </div>
                ) : (
                  <div key={i} className={`g-ev${e.live ? " g-ev--live" : ""}`}>
                    <span className="at num">{e.at}</span>
                    <span className="tool">{e.tool}</span>
                    <span className="tg">{e.target}{e.live ? <span className="g-cursor" aria-hidden="true" /> : null}</span>
                    <span>
                      {e.added !== undefined ? <DiffCount added={e.added} removed={e.removed} /> : null}
                      {e.exit !== undefined ? <span className={`exit ${e.exit === 0 ? "ok" : "bad"}`}>exit {e.exit}</span> : null}
                    </span>
                    {e.out ? <div className="out">{e.out.map((o, j) => <div key={j} className={o.tone}>{o.text}</div>)}</div> : null}
                  </div>
                ),
              )}
            </div>
          </section>

          <aside className="g-review" aria-label="Review">
            <div>
              <h3>Files changed <span className="n">{backendFiles.length}</span><span className="r"><DiffCount added={sel.added} removed={sel.removed} /></span></h3>
              {backendFiles.map((f) => (
                <div key={f.path} className="g-file">
                  <IcFile />
                  <span className="p">{split(f.path)}</span>
                  {f.state === "Writing" ? <span className="st">Writing</span> : <DiffCount added={f.added} removed={f.removed} />}
                </div>
              ))}
            </div>
            <div>
              <h3>Commands run <span className="n">{backendCommands.length}</span></h3>
              {backendCommands.map((c) => (
                <div key={c.at} className="g-cmd">
                  <IcTerminal />
                  <span className="p">{c.cmd}</span>
                  <span className={`g-exit ${c.exit === 0 ? "ok" : "bad"}`}>exit {c.exit}</span>
                  <span className="s">{c.at}, {c.summary}</span>
                </div>
              ))}
            </div>
            <div>
              <h3>Acceptance <span className="n">Checked in Verify</span></h3>
              {criteria.map((c) => (
                <div key={c.id} className="g-crit">
                  <IcTarget />
                  <span className="tx">{c.text}</span>
                  <span className="id">{c.id}</span>
                </div>
              ))}
            </div>
          </aside>
        </div>
      </div>
    </ShellG>
  );
}
