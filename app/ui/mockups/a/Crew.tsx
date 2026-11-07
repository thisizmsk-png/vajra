import { ShellA } from "./Shell";
import { DiffCount, Kbd, StatusDot } from "../shared";
import { agents, approval, backendStream, criteria, phases, run } from "../data";

export function CrewA() {
  const sel = agents.find((a) => a.id === "backend")!;
  return (
    <ShellA
      current="crew"
      bar={
        <>
          <div className="ctx"><b>{sel.role}</b> <span className="muted">{sel.taskId} {sel.task}</span></div>
          <div className="acts">
            <button className="act act--primary">Approve plan <span className="a-key"><Kbd>⌘</Kbd><Kbd>↵</Kbd></span></button>
            <span className="sep" />
            <button className="act">Message <Kbd>M</Kbd></button>
            <button className="act">Redirect <Kbd>R</Kbd></button>
            <button className="act">Stop worker <span className="a-key"><Kbd>⌘</Kbd><Kbd>.</Kbd></span></button>
            <span className="sep" />
            <button className="act">Actions <span className="a-key"><Kbd>⌘</Kbd><Kbd>K</Kbd></span></button>
          </div>
        </>
      }
    >
      <header className="a-head">
        <div className="a-crumb"><span className="muted">Crew</span><span className="muted">/</span>{run.title}</div>
        <div className="a-phases" aria-label="Phases">
          {phases.map((p) => <span key={p.name} className={`a-ph a-ph--${p.state}`}>{p.name}</span>)}
        </div>
        <div className="r">
          <span className="muted num">Round {run.round} of {run.maxRounds}, {run.elapsed}</span>
          <button className="a-btn">Stop run</button>
        </div>
      </header>

      <div className="a-gate" role="region" aria-label="Waiting for you">
        <StatusDot status="needs_approval" label="Plan change" />
        <div className="txt">
          <div><b>{approval.title}</b> <span className="muted">{approval.from}, {approval.at}</span></div>
          <div className="why">{approval.why}</div>
        </div>
        <div className="acts">
          <button className="a-btn a-btn--ghost">View plan v2</button>
          <button className="a-btn">Reject</button>
          <button className="a-btn a-btn--primary">Approve plan <span className="a-key"><Kbd>⌘</Kbd><Kbd>↵</Kbd></span></button>
        </div>
      </div>

      <div className="a-body">
        <div className="a-crew">
          <div className="lbl"><span>Crew</span><span className="num">4 working, 1 waiting, 1 done</span></div>
          <div role="tree" aria-label="Crew">
            {agents.map((a) => (
              <div key={a.id} role="treeitem" aria-selected={a.id === sel.id} className={`a-ag${a.id === "backend-1" ? " sub" : ""}`} tabIndex={a.id === sel.id ? 0 : -1}>
                <span className="n">{a.role}{a.taskId ? <span className="tid">{a.taskId}</span> : null}</span>
                <StatusDot status={a.status} label={a.statusLabel} />
                <span className="task">{a.task}</span>
                <span className="last"><span>{a.last}</span><span>{a.added !== undefined ? <DiffCount added={a.added} removed={a.removed} /> : a.lastAt}</span></span>
              </div>
            ))}
          </div>
        </div>

        <section className="a-pane" aria-label={`${sel.role} worker`}>
          <div className="a-panehead">
            <span className="who">{sel.role}</span>
            <StatusDot status={sel.status} />
            <span className="muted">{sel.taskId} {sel.task}</span>
          </div>
          <div className="a-tabs" role="tablist" aria-label="Worker views">
            <button role="tab" aria-selected="true" className="a-tab">Stream</button>
            <button role="tab" aria-selected="false" className="a-tab">Diff <span className="c">{sel.files} files</span></button>
            <button role="tab" aria-selected="false" className="a-tab">Task</button>
            <button role="tab" aria-selected="false" className="a-tab">Worktree</button>
          </div>
          <div className="a-stream" role="log" aria-label="Backend stream, read only" aria-live="polite">
            {backendStream.map((e, i) =>
              e.kind === "action" ? (
                <div key={i} className="a-card" role="article">
                  <div className="h"><b>{e.from} sent a message</b><span>{e.at}</span><span className="st">{e.state}</span></div>
                  {e.text}
                </div>
              ) : (
                <div key={i} className={`a-ev${e.live ? " a-ev--live" : ""}`}>
                  <span className="at num">{e.at}</span>
                  <span className="tool">{e.tool}</span>
                  <span className="tg">{e.target}</span>
                  <span>
                    {e.added !== undefined ? <DiffCount added={e.added} removed={e.removed} /> : null}
                    {e.exit !== undefined ? <span className={`exit ${e.exit === 0 ? "ok" : "bad"}`}>exit {e.exit}</span> : null}
                    {e.live ? <span className="muted">now</span> : null}
                  </span>
                  {e.out ? (
                    <div className="out">
                      {e.out.map((o, j) => <div key={j} className={o.tone}>{o.text}</div>)}
                    </div>
                  ) : null}
                </div>
              ),
            )}
          </div>
        </section>

        <aside className="a-insp" aria-label="Details">
          <div>
            <h3>Task</h3>
            <dl className="a-kv">
              <dt>Task</dt><dd>{sel.taskId} {sel.task}</dd>
              <dt>Depends on</dt><dd>t1 Architect, done</dd>
              <dt>Blocks</dt><dd>t5 Verifier</dd>
              <dt>Attempt</dt><dd className="num">1 of 3</dd>
            </dl>
          </div>
          <div>
            <h3>Worktree</h3>
            <dl className="a-kv">
              <dt>Branch</dt><dd className="mono">vajra/{run.id}/t2</dd>
              <dt>Path</dt><dd className="mono">.claude/worktrees/t2</dd>
              <dt>Changes</dt><dd><DiffCount added={sel.added} removed={sel.removed} /> <span className="muted">in {sel.files} files</span></dd>
            </dl>
          </div>
          <div>
            <h3>Acceptance, not run yet</h3>
            <div className="a-crit">
              {criteria.map((c) => <div key={c.id}><span className="id">{c.id}</span><span className="sec">{c.text}</span></div>)}
            </div>
          </div>
        </aside>
      </div>
    </ShellA>
  );
}
