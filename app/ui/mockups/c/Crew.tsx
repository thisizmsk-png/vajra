import { ShellC } from "./Shell";
import { DiffCount, Kbd, StatusDot } from "../shared";
import { agents, approval, backendStream, run, score } from "../data";

// The score spans 21:40 to 22:16; "now" is minute 27 (22:07).
const SPAN = 36;
const NOW = 27;
const pct = (m: number) => `${(m / SPAN) * 100}%`;
const TICKS = [0, 5, 10, 15, 20, 25, 30, 35].map((m) => ({ m, label: `${21 + Math.floor((40 + m) / 60)}:${String((40 + m) % 60).padStart(2, "0")}` }));

export function CrewC() {
  const sel = "backend";
  return (
    <ShellC current="crew">
      <main className="c-crew" aria-labelledby="c-run-t">
        <div className="c-title">
          <h1 id="c-run-t">{run.title}</h1>
          <span className="ph">Understand, Plan, <b>Execute round {run.round} of {run.maxRounds}</b>, then Verify and Deliver. Started {run.started}.</span>
          <span className="r">
            <button className="c-btn">Message Lead <Kbd>⌘L</Kbd></button>
            <button className="c-btn">Stop run</button>
          </span>
        </div>

        <section className="c-score" aria-label="Run score: what each agent did over time">
          <div className="c-axis" aria-hidden="true">
            <span />
            <div className="ticks">{TICKS.map((t) => <span key={t.m} style={{ left: pct(t.m) }}>{t.label}</span>)}</div>
            <span />
          </div>
          <div role="tree" aria-label="Crew">
            {agents.map((a) => (
              <div key={a.id} className="c-lane" role="treeitem" aria-selected={a.id === sel} tabIndex={a.id === sel ? 0 : -1}>
                <span className="who">{a.role}<span className="tid">{a.taskId}</span></span>
                <span className="c-track">
                  {score[a.id].map((s, i) => (
                    <span key={i} className={`c-seg c-seg--${s.kind}`} style={{ left: pct(s.from), width: pct(s.to - s.from) }} />
                  ))}
                  {a.id === "verifier" || a.id === "security" ? (
                    <span className="c-wait" style={{ left: `calc(${pct(NOW)} + 8px)`, right: 0 }}>{a.id === "verifier" ? "waits on t2, t4" : "starts when you approve"}</span>
                  ) : null}
                </span>
                <span className="end">
                  {a.added !== undefined && a.status !== "done" ? <DiffCount added={a.added} removed={a.removed} /> : <StatusDot status={a.status} label={a.status === "in_progress" ? "Live" : a.status === "needs_approval" ? "Proposed" : a.status === "ready" ? "Ready" : "Done"} />}
                </span>
              </div>
            ))}
          </div>
          <div className="c-future" style={{ left: `calc(212px + (100% - 212px - 136px) * ${NOW / SPAN})` }} aria-hidden="true" />
          <div className="c-gate" style={{ left: `calc(212px + (100% - 212px - 136px) * ${NOW / SPAN})` }} aria-hidden="true">
            <span className="c-gatetag">Now 22:07, gate: plan v2</span>
          </div>
          <div className="c-legend" aria-hidden="true">
            <span><i className="sw-read" />Reading</span>
            <span><i className="sw-edit" />Editing or planning</span>
            <span><i className="tick sw-pass" />Tests pass</span>
            <span><i className="tick sw-fail" />Tests fail</span>
            <span><i className="tick sw-msg" />Lead message</span>
          </div>
        </section>

        <div className="c-below">
          <section className="c-gatecard" aria-labelledby="c-gate-h">
            <div className="hd">
              <div className="k"><span>{approval.kind}, from {approval.from} at {approval.at}</span><span>Needs you</span></div>
              <h2 id="c-gate-h">{approval.title}</h2>
            </div>
            <div className="bd">
              <span>{approval.why}</span>
              <div className="ch">
                {approval.changes.map((c) => <span key={c.text}><span className={c.sign === "+" ? "p" : "t"}>{c.sign}</span> {c.text}</span>)}
              </div>
              <dl className="c-ifs">
                <dt>If you approve</dt><dd>{approval.ifApprove}</dd>
                <dt>If you reject</dt><dd>{approval.ifReject}</dd>
              </dl>
            </div>
            <div className="ft">
              <button className="c-btn">Reject</button>
              <button className="c-btn">Ask Lead</button>
              <button className="c-btn c-btn--bolt">Approve plan <Kbd>⌘↵</Kbd></button>
            </div>
          </section>

          <section className="c-pane" aria-label="Backend worker">
            <div className="c-panehd">
              <span className="who">Backend</span>
              <StatusDot status="in_progress" label="Live" />
              <span className="c-tabs" role="tablist" aria-label="Worker views">
                <button role="tab" aria-selected="true" className="c-tab">Stream</button>
                <button role="tab" aria-selected="false" className="c-tab">Diff</button>
                <button role="tab" aria-selected="false" className="c-tab">Task</button>
                <button role="tab" aria-selected="false" className="c-tab">Worktree</button>
              </span>
              <span className="r">
                <button className="c-btn">Message</button>
                <button className="c-btn">Stop <Kbd>⌘.</Kbd></button>
              </span>
            </div>
            <div className="c-stream" role="log" aria-label="Backend stream, read only">
              {backendStream.map((e, i) =>
                e.kind === "action" ? (
                  <div key={i} className="c-msg"><div className="h">{e.from} to Backend, {e.at}, {e.state.toLowerCase()}</div>{e.text}</div>
                ) : (
                  <div key={i} className={`c-ev${e.live ? " c-ev--live" : ""}`}>
                    <span className="at num">{e.at}</span>
                    <span className="tool">{e.tool}</span>
                    <span className="tg">{e.target}</span>
                    <span>{e.added !== undefined ? <DiffCount added={e.added} removed={e.removed} /> : null}{e.exit !== undefined ? <span className={e.exit ? "fail" : "pass"}>exit {e.exit}</span> : null}{e.live ? <span className="muted">now</span> : null}</span>
                    {e.out ? <div className="out">{e.out.map((o, j) => <div key={j} className={o.tone}>{o.text}</div>)}</div> : null}
                  </div>
                ),
              )}
            </div>
          </section>
        </div>
      </main>
    </ShellC>
  );
}
