import { ShellC } from "./Shell";
import { Kbd, StatusDot } from "../shared";
import { approval, ask, history, preflight, repo, run } from "../data";

const outcome = { Delivered: "done", Stopped: "blocked", Failed: "failed" } as const;

export function HomeC() {
  return (
    <ShellC current="home">
      <div className="c-home">
        <main className="c-compose" aria-labelledby="c-h">
          <h1 id="c-h" className="c-h1">What should the crew build?</h1>
          <p className="c-ask" role="textbox" aria-labelledby="c-h" contentEditable suppressContentEditableWarning>{ask}</p>
          <p className="c-sentence">
            In <button className="c-val">{repo.name}</button>, from <button className="c-val">{repo.base} at {repo.head}</button>,
            with up to <button className="c-val">3 rounds</button> and <button className="c-val">4 workers</button> at once.
            Nothing gets pushed.
          </p>
          <div className="c-go">
            <button className="c-btn c-btn--bolt c-btn--big">Start run <Kbd>⌘↵</Kbd></button>
            <span className="muted">The Lead writes a plan first. You approve it before anyone edits code.</span>
          </div>
          <div className="c-checks" aria-label="Checks">
            {preflight.map((p) => (
              <div key={p.name} className="item">
                <StatusDot status={p.status === "pass" ? "pass" : "blocked"} label={`${p.name}: ${p.label}`} />
                <span className="muted">{p.detail}</span>
              </div>
            ))}
          </div>
        </main>
        <aside className="c-side" aria-label="Runs">
          <section className="c-live" aria-labelledby="c-live-t">
            <div className="k"><span>Running now</span><span className="num">{run.elapsed}</span></div>
            <div className="t" id="c-live-t">{run.title}</div>
            <div className="s">{run.phase}, round {run.round} of {run.maxRounds}. {run.workers} workers live, 1 done.</div>
            <div className="gate">
              <span><b>Waiting at the gate.</b> Plan v2: {approval.title.toLowerCase()}.</span>
              <button className="c-btn">Open</button>
            </div>
          </section>
          <section aria-labelledby="c-recent">
            <h2 className="c-h2" id="c-recent">Recent runs <a href="#history">All history</a></h2>
            <div className="c-runs">
              {history.map((h) => (
                <div key={h.id} className="c-run">
                  <span className="t">{h.title}</span>
                  <span className="s">{h.when}, {h.detail}</span>
                  <span className="o">
                    <StatusDot status={outcome[h.outcome]} label={h.outcome} />
                    <div className="muted num">{h.passing} pass</div>
                  </span>
                </div>
              ))}
            </div>
          </section>
        </aside>
      </div>
    </ShellC>
  );
}
