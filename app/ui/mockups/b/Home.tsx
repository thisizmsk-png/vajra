import { ShellB } from "./Shell";
import { Kbd } from "../shared";
import { approval, ask, history, preflight, repo, run } from "../data";

const bar = { Delivered: "ok", Stopped: "stop", Failed: "bad" } as const;

export function HomeB() {
  return (
    <ShellB current="home">
      <main className="b-home" aria-labelledby="b-h">
        <h1 id="b-h" className="m-sr">Start a run</h1>
        <div className="b-block b-block--focus">
          <div className="b-ctx"><span className="path">{repo.path}</span><span className="br">{repo.base}</span><span>{repo.head}</span><span style={{ marginLeft: "auto" }}>clean</span></div>
          <label className="b-prompt">
            <span className="ps" aria-hidden="true">❯</span>
            <span className="cmd">vajra deliver</span>
            <span className="ask" role="textbox" aria-label="What should the crew do?" contentEditable suppressContentEditableWarning>{ask}</span>
          </label>
          <div className="b-flags">
            <button className="b-flag"><span className="k">--base</span><span className="v">{repo.base}</span></button>
            <button className="b-flag"><span className="k">--rounds</span><span className="v">3</span></button>
            <button className="b-flag"><span className="k">--workers</span><span className="v">4</span></button>
            <button className="b-flag"><span className="k">--repo</span><span className="v">{repo.name}</span></button>
            <span className="hint">Tab to edit a flag</span>
            <button className="b-btn b-btn--primary">Run <Kbd>⌘↵</Kbd></button>
          </div>
        </div>

        <div className="b-two">
          <section className="b-block" aria-labelledby="b-pre">
            <div className="b-cmdline"><span className="ps">$</span><span id="b-pre">vajra doctor</span><span className="r">21:38, exit 0</span></div>
            <div className="b-out">
              {preflight.map((p) => (
                <div key={p.name} className="b-line">
                  <span className={p.status === "pass" ? "pass" : "sec"}>{p.status === "pass" ? "✓ pass" : "▲ off"}</span>
                  <span>{p.name}</span>
                  <span className="muted">{p.detail}</span>
                </div>
              ))}
            </div>
          </section>

          <section className="b-block" aria-labelledby="b-live">
            <div className="b-cmdline"><span className="ps live">●</span><span id="b-live">{run.title}</span><span className="r">{run.elapsed}</span></div>
            <div className="b-out">
              <div className="b-line"><span className="muted">phase</span><span>{run.phase.toLowerCase()} r{run.round}/{run.maxRounds}</span><span className="muted">{run.workers} workers live, 1 done</span></div>
              <div className="b-line"><span className="muted">needs you</span><span className="sec">▲ plan v2</span><span>{approval.title.toLowerCase()}</span></div>
              <div className="b-line"><span className="muted">id</span><span className="muted" style={{ gridColumn: "2 / -1" }}>{run.id}</span></div>
              <div style={{ display: "flex", gap: 8, marginTop: 10 }}>
                <button className="b-btn">Open crew <Kbd>⌘2</Kbd></button>
                <button className="b-btn">Review plan</button>
              </div>
            </div>
          </section>
        </div>

        <section className="b-block" aria-labelledby="b-hist" style={{ maxWidth: 1040 }}>
          <div className="b-cmdline"><span className="ps">$</span><span id="b-hist">vajra history --repo {repo.name}</span><span className="r">{history.length} runs</span></div>
          <div className="b-out b-hist" style={{ paddingLeft: 0 }}>
            {history.map((h) => (
              <div key={h.id} className="b-hrow">
                <span className={`bar ${bar[h.outcome]}`} aria-hidden="true" />
                <span className="t">{h.title}</span>
                <span className="s">{h.id}  {h.outcome.toLowerCase()}, {h.detail}, {h.when}</span>
                <span className="side">
                  <span className={h.outcome === "Delivered" ? "pass" : h.outcome === "Failed" ? "fail" : "sec"}>{h.passing} pass</span>
                  {h.resumable ? <button className="b-btn">Resume</button> : <button className="b-btn">Report</button>}
                </span>
              </div>
            ))}
          </div>
        </section>
      </main>
    </ShellB>
  );
}
