import { ShellB } from "./Shell";
import { DiffCount, Kbd, StatusDot } from "../shared";
import { agents, approval, backendStream, run, tails } from "../data";

function Lane({ id, main, sel }: { id: string; main?: boolean; sel?: boolean }) {
  const a = agents.find((x) => x.id === id)!;
  return (
    <section className={`b-lane${main ? " b-lane--main" : ""}${sel ? " b-lane--sel" : ""}`} aria-label={`${a.role} lane`}>
      <div className="b-lanehead">
        <span className="who">{a.role}</span>
        <StatusDot status={a.status} label={a.statusLabel ?? "live"} />
        <span className="task">{a.taskId} {a.task}</span>
        <span className="r">
          {main ? (
            <span className="b-ltabs" role="tablist" aria-label="Worker views">
              <span role="tab" aria-selected="true">stream</span>
              <span role="tab" aria-selected="false">diff</span>
              <span role="tab" aria-selected="false">task</span>
              <span role="tab" aria-selected="false">worktree</span>
            </span>
          ) : null}
          <DiffCount added={a.added} removed={a.removed} />
        </span>
      </div>
      <div className="b-lanebody" role="log" aria-label={`${a.role} output, read only`}>
        {main
          ? backendStream.map((e, i) =>
              e.kind === "action" ? (
                <div key={i} className="b-blk b-blk--msg">
                  <div className="h"><span className="at">{e.at}</span><span className="tool">lead</span><span className="tg">message to backend</span><span className="muted">{e.state.toLowerCase()}</span></div>
                  <div className="o">{e.text}</div>
                </div>
              ) : (
                <div key={i} className={`b-blk${e.live ? " b-blk--live" : e.exit === 1 ? " b-blk--bad" : e.exit === 0 ? " b-blk--ok" : ""}`}>
                  <div className="h">
                    <span className="at">{e.at}</span>
                    <span className="tool">{e.tool.toLowerCase()}</span>
                    <span className="tg">{e.tool === "Bash" ? `$ ${e.target}` : e.target}{e.live ? <span className="b-cur" aria-hidden="true" /> : null}</span>
                    <span>
                      {e.added !== undefined ? <DiffCount added={e.added} removed={e.removed} /> : null}
                      {e.exit !== undefined ? <span className={e.exit === 0 ? "pass" : "fail"}>exit {e.exit}</span> : null}
                      {e.live ? <span className="live">writing</span> : null}
                    </span>
                  </div>
                  {e.out?.map((o, j) => <div key={j} className={`o ${o.tone ?? ""}`}>{o.text}</div>)}
                </div>
              ),
            )
          : tails[id].map((t, i) => (
              <div key={i} className="b-tl">
                <span className="at">{t.at}</span>
                <span className={t.tone === "live" ? "live" : t.tone}>{t.text}{t.tone === "live" ? <span className="b-cur" aria-hidden="true" /> : null}</span>
              </div>
            ))}
      </div>
    </section>
  );
}

export function CrewB() {
  return (
    <ShellB current="crew">
      <main className="b-crew" aria-label="Crew">
        <div className="b-runhead">
          <span className="t">{run.title}</span>
          <span className="ph">understand ✓  plan ✓  <b>execute r{run.round}</b>  verify  deliver</span>
          <span className="muted">{run.started} to now, {run.elapsed}</span>
          <span className="r">
            <button className="b-btn">Message <Kbd>M</Kbd></button>
            <button className="b-btn">Stop worker <Kbd>⌘.</Kbd></button>
            <button className="b-btn">Stop run</button>
          </span>
        </div>
        <div className="b-lanes">
          <Lane id="backend" main sel />
          <Lane id="frontend" />
          <Lane id="backend-1" />
        </div>
        <div className="b-waiting" aria-label="Not running">
          <StatusDot status="done" label="Architect t1 done 21:52" />
          <StatusDot status="ready" label="Verifier t5 waits on t2, t4" />
          <StatusDot status="needs_approval" label="Security t6 needs your approval" />
        </div>
        <div>
          <div className="b-approve" role="region" aria-label="Waiting for you">
            <div className="h"><b>Approve plan v2: {approval.title.toLowerCase()}</b><span>{approval.from.toLowerCase()} {approval.at}</span></div>
            <div className="why">{approval.why}</div>
            <div className="ch">
              {approval.changes.map((c) => <span key={c.text}><span className={c.sign === "+" ? "p" : "t"}>{c.sign}</span> {c.text}</span>)}
            </div>
            <div className="acts">
              <button className="b-btn">Reject <Kbd>⌫</Kbd></button>
              <button className="b-btn b-btn--primary">Approve <Kbd>⌘↵</Kbd></button>
            </div>
          </div>
          <label className="b-leadp">
            <span className="ps" aria-hidden="true">lead ❯</span>
            <span className="ph">Ask the Lead, or type / for commands</span>
            <span className="r"><span>/message</span><span>/redirect</span><span>/stop</span><Kbd>⌘L</Kbd></span>
          </label>
        </div>
      </main>
    </ShellB>
  );
}
