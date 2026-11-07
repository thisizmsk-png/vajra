import { Act, ShellG } from "./Shell";
import { Kbd, StatusDot } from "../shared";
import { ago, approval, history, preflight, question, repo, run, templates } from "../data";
import { IcAt, IcBranch, IcChevron, IcFile, IcShield, IcTerminal } from "./icons";

const outcome = { Delivered: "done", Stopped: "blocked", Failed: "failed" } as const;

function Launcher({ empty }: { empty?: boolean }) {
  return (
    <>
      <div className="g-launch">
        <div className="g-ask" role="textbox" aria-multiline="true" aria-label="What should the crew do?" contentEditable suppressContentEditableWarning>
          {empty ? (
            <span className="ph">Describe a change, for example: add pagination to the webhooks list</span>
          ) : (
            <>
              Add per-key rate limiting to the public API in
              <span className="g-mention" contentEditable={false}><IcFile />src/middleware</span>
              , with a 429 body and a Retry-After header
            </>
          )}
        </div>
        <div className="hint">
          <span><IcAt /> Add files or folders</span>
          <span><Kbd>/</Kbd> Pick a crew</span>
          <span><Kbd>⇧</Kbd><Kbd>↵</Kbd> New line</span>
        </div>
        <div className="row">
          <button className="g-tok"><span className="k">Base</span><IcBranch />{repo.base} <span className="mono">{repo.head}</span><IcChevron /></button>
          <button className="g-tok"><span className="k">Rounds</span>3<IcChevron /></button>
          <button className="g-tok"><span className="k">Workers</span>4<IcChevron /></button>
          <button className="g-tok"><IcShield />Guard on<IcChevron /></button>
          <button className="g-btn g-btn--primary">Start run <span className="g-keys"><Kbd>⌘</Kbd><Kbd>↵</Kbd></span></button>
        </div>
      </div>
      <div className="g-pre" role="status">
        <StatusDot status="pass" label="3 of 4 checks pass" />
        <span>OS sandbox is off. Turn it on before unattended runs.</span>
        <a className="lnk" href="#settings">Fix in Settings</a>
      </div>
    </>
  );
}

export function HomeG({ empty = false }: { empty?: boolean }) {
  return (
    <ShellG
      current="home"
      inbox={empty ? 0 : 2}
      empty={empty}
      bar={
        <>
          <div className="ctx"><b>{repo.name}</b><span className="muted">{repo.path}</span></div>
          <div className="acts">
            <Act label="Start run" keys={["⌘", "↵"]} primary />
            <span className="sep" />
            {empty ? null : <Act label="Open inbox" keys={["⌘", "2"]} />}
            <Act label="Switch repository" keys={["⌘", "O"]} />
            <span className="sep" />
            <Act label="All actions" keys={["⌘", "K"]} />
          </div>
        </>
      }
    >
      <div className="g-home">
        <div className="g-launchcol">
          <h1 className="g-h1">Start a run</h1>
          <p className="g-lede">The Lead plans first and you approve the plan. The crew builds in worktrees. Nothing is pushed.</p>
          <Launcher empty={empty} />
        </div>

        {empty ? (
          <div className="g-cols">
            <section className="g-sec" aria-labelledby="g-tpl">
              <h2 id="g-tpl">Crews you can start</h2>
              {templates.map((t) => (
                <div key={t.name} className="g-li g-li--tpl" role="button" tabIndex={0}>
                  <IcTerminal />
                  <span className="t">{t.name}</span>
                  <span className="s">{t.cmd}</span>
                  <span className="side">{t.hint}</span>
                </div>
              ))}
            </section>
            <section className="g-sec" aria-labelledby="g-rec0">
              <h2 id="g-rec0">Recent runs</h2>
              <p className="g-empty"><b>No runs in {repo.name} yet.</b> Your first run shows up here and in the sidebar while it works.</p>
            </section>
          </div>
        ) : (
          <div className="g-cols">
            <div>
              <section className="g-sec" aria-labelledby="g-needs">
                <h2 id="g-needs">Needs you <span className="n">2</span><a className="r" href="#inbox">Open inbox</a></h2>
                <div className="g-li g-dot" aria-selected="true">
                  <StatusDot status="needs_approval" label="Plan change" />
                  <span className="t">{approval.title}</span>
                  <span className="s">Plan change from {approval.from}, {run.title}, 3 min ago</span>
                  <span className="side"><button className="g-btn g-btn--sm">Review</button></span>
                </div>
                <div className="g-li g-dot">
                  <StatusDot status="question" label="Question" />
                  <span className="t">{question.title}</span>
                  <span className="s">Question from {question.from}, {run.title}, 1 min ago</span>
                  <span className="side"><button className="g-btn g-btn--sm">Answer</button></span>
                </div>
              </section>
              <section className="g-sec" aria-labelledby="g-now">
                <h2 id="g-now">Running <span className="n">1</span></h2>
                <div className="g-li g-dot g-live">
                  <StatusDot status="in_progress" label="Running" />
                  <span className="t">{run.title}</span>
                  <span className="s">{run.phase}, round {run.round} of {run.maxRounds}. 3 working, 2 need you, 1 waiting, 1 done. Started {run.started}.</span>
                  <span className="side"><button className="g-btn g-btn--sm">Open crew</button></span>
                </div>
              </section>
            </div>
            <section className="g-sec" aria-labelledby="g-rec">
              <h2 id="g-rec">Recent runs <a className="r" href="#history">All history</a></h2>
              {history.map((h) => (
                <div key={h.id} className="g-li g-dot">
                  <StatusDot status={outcome[h.outcome]} label={h.outcome} />
                  <span className="t">{h.title}</span>
                  <span className="s">{h.outcome}, {h.detail}</span>
                  <span className="side">
                    <span className="pass">{h.passing} pass</span>
                    {h.resumable ? <button className="g-btn g-btn--sm">Resume</button> : <span>{ago[h.id]} ago</span>}
                  </span>
                </div>
              ))}
            </section>
          </div>
        )}
      </div>
    </ShellG>
  );
}
