import { ShellA } from "./Shell";
import { Chevron, Kbd, StatusDot } from "../shared";
import { approval, ask, history, preflight, repo, run } from "../data";

export function HomeA() {
  const warn = preflight.find((p) => p.status !== "pass");
  return (
    <ShellA
      current="home"
      bar={
        <>
          <div className="ctx"><b>{repo.name}</b> <span className="muted">on {repo.base}</span></div>
          <div className="acts">
            <button className="act act--primary">Start run <span className="a-key"><Kbd>⌘</Kbd><Kbd>↵</Kbd></span></button>
            <span className="sep" />
            <button className="act">Choose repository <span className="a-key"><Kbd>⌘</Kbd><Kbd>O</Kbd></span></button>
            <span className="sep" />
            <button className="act">Actions <span className="a-key"><Kbd>⌘</Kbd><Kbd>K</Kbd></span></button>
          </div>
        </>
      }
    >
      <div className="a-home">
        <div className="a-col">
          <h1 className="a-h1">Start a run</h1>
          <p className="a-lede">Describe the change. The Lead plans it, the crew builds it in worktrees, and nothing is pushed.</p>

          <div className="a-launch">
            <label htmlFor="a-ask" className="m-sr">What should the crew do?</label>
            <textarea id="a-ask" defaultValue={ask} />
            <div className="row">
              <button className="a-tok"><span className="k">Repo</span>{repo.name}<Chevron /></button>
              <button className="a-tok"><span className="k">Base</span>{repo.base} <span className="mono">{repo.head}</span><Chevron /></button>
              <button className="a-tok"><span className="k">Rounds</span>3<Chevron /></button>
              <button className="a-tok"><span className="k">Workers</span>4<Chevron /></button>
              <button className="a-btn a-btn--primary">Start run <span className="a-key"><Kbd>⌘</Kbd><Kbd>↵</Kbd></span></button>
            </div>
          </div>
          <div className="a-pre" role="status">
            <StatusDot status="pass" label="3 of 4 checks pass" />
            {warn ? <span className="warn">{warn.name}: {warn.detail}</span> : null}
            <a className="lnk" href="#preflight">Details</a>
          </div>

          <section className="a-sec" aria-labelledby="a-now-h">
            <div className="a-sech"><h2 id="a-now-h">Running now</h2></div>
            <div className="a-now">
              <div className="t">{run.title}</div>
              <div className="meta">
                <StatusDot status="in_progress" label={`${run.phase}, round ${run.round} of ${run.maxRounds}`} />
                <span className="num">{run.workers} workers</span>
                <span className="num">Started {run.started}, {run.elapsed}</span>
                <span className="mono">{run.id}</span>
              </div>
              <div className="acts">
                <button className="a-btn">Open crew <span className="a-key"><Kbd>⌘</Kbd><Kbd>2</Kbd></span></button>
              </div>
              <div className="ask">
                <StatusDot status="needs_approval" label="Needs you" />
                <span><b>{approval.title}.</b> <span className="sec">{approval.from} proposed plan v2 at {approval.at}.</span></span>
                <button className="a-btn" style={{ marginLeft: "auto" }}>Review plan</button>
              </div>
            </div>
          </section>

          <section className="a-sec" aria-labelledby="a-hist-h">
            <div className="a-sech"><h2 id="a-hist-h">Recent runs</h2><span className="muted">{repo.name}</span><a className="r sec" href="#history">All history <Kbd>⌘</Kbd><Kbd>5</Kbd></a></div>
            <div className="a-list" role="listbox" aria-label="Recent runs">
              {history.map((h, i) => (
                <div key={h.id} className="a-lrow" role="option" aria-selected={i === 1}>
                  <span className="t">{h.title}<span className="id mono">{h.id}</span></span>
                  <span className={`o--${h.outcome}`}>{h.outcome}{h.outcome !== "Failed" ? <span className="muted">, {h.detail.split(",")[0]}</span> : null}</span>
                  <span className="num sec">{h.passing} pass</span>
                  <span className="num muted">{h.when}</span>
                  <span className="r">{h.resumable ? <button className="a-btn a-btn--ghost">Resume</button> : <span className="muted">Report</span>}</span>
                </div>
              ))}
            </div>
          </section>
        </div>
      </div>
    </ShellA>
  );
}
