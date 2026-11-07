import { Act, ShellG } from "./Shell";
import { StatusDot } from "../shared";
import { agents, approval, inboxDone, question, run } from "../data";
import { IcCheck } from "./icons";

/** Inbox: Linear-style triage. Every request from every run, newest first; the selected one opens on the right. */
export function InboxG() {
  const plan = agents.filter((a) => a.taskId && a.id !== "security");
  return (
    <ShellG
      current="inbox"
      bar={
        <>
          <div className="ctx"><b>Plan change</b><span className="muted">{run.title}</span></div>
          <div className="acts">
            <Act label="Approve plan" keys={["⌘", "↵"]} primary />
            <Act label="Reject" keys={["⌘", "⌫"]} />
            <span className="sep" />
            <Act label="Next" keys={["J"]} />
            <Act label="Previous" keys={["K"]} />
            <span className="sep" />
            <Act label="Open run" keys={["⌘", "O"]} />
          </div>
        </>
      }
    >
      <div className="g-inbox">
        <div className="g-ilist">
          <div className="top">
            <h1>Inbox</h1>
            <div className="g-seg" role="radiogroup" aria-label="Show">
              <button role="radio" aria-checked="true">Needs you</button>
              <button role="radio" aria-checked="false">All</button>
            </div>
          </div>
          <div className="g-irows" role="listbox" aria-label="Requests">
            <div className="g-grp"><span>Needs you</span><span className="num">2</span></div>
            <div className="g-irow g-dot" role="option" aria-selected="true" tabIndex={0}>
              <StatusDot status="needs_approval" label="Plan change" />
              <span className="t">{approval.title}</span>
              <span className="r num">{approval.at}</span>
              <span className="s">Plan change from {approval.from} in {run.title}</span>
            </div>
            <div className="g-irow g-dot" role="option" aria-selected="false" tabIndex={-1}>
              <StatusDot status="question" label="Question" />
              <span className="t">{question.title}</span>
              <span className="r num">{question.at}</span>
              <span className="s">Question from {question.from} in {run.title}</span>
            </div>
            <div className="g-grp"><span>Done today</span><span className="num">{inboxDone.length}</span></div>
            {inboxDone.map((d) => (
              <div key={d.title} className="g-irow g-irow--done" role="option" aria-selected="false" tabIndex={-1}>
                <span className="muted"><IcCheck /></span>
                <span className="t">{d.title}</span>
                <span className="r num">{d.at}</span>
                <span className="s">{d.who}</span>
              </div>
            ))}
          </div>
        </div>

        <section className="g-idetail" aria-label={approval.title}>
          <div className="scroll">
            <div className="col">
              <div className="g-kind g-dot"><StatusDot status="needs_approval" label="Plan change" />Plan change, plan v1 to v2</div>
              <h2>{approval.title}</h2>
              <div className="meta"><span>From {approval.from} at {approval.at}</span><span>{run.title}</span><span>Round {run.round} of {run.maxRounds}</span></div>
              <p>{approval.why}</p>

              <h3>Plan v2</h3>
              <table className="g-plan">
                <thead><tr><th aria-label="Change" /><th>Task</th><th>Owner and work</th></tr></thead>
                <tbody>
                  <tr>
                    <td className="sg p" aria-label="Added">+</td><td className="id">t6</td>
                    <td>Security reviews key handling in the limiter</td>
                  </tr>
                  <tr>
                    <td className="sg t" aria-label="Changed">~</td><td className="id">t5</td>
                    <td>Verifier now waits on t6 as well as t2 and t4</td>
                  </tr>
                  {plan.filter((a) => a.taskId !== "t5").map((a) => (
                    <tr key={a.id} className="dim">
                      <td className="sg e" aria-label="Unchanged">=</td><td className="id">{a.taskId}</td>
                      <td>{a.role}: {a.task}</td>
                    </tr>
                  ))}
                </tbody>
              </table>

              <h3>What happens next</h3>
              <dl className="g-ifs">
                <dt>If you approve</dt><dd>{approval.ifApprove}</dd>
                <dt>If you reject</dt><dd>{approval.ifReject}</dd>
                <dt>Cost</dt><dd>{approval.cost}</dd>
              </dl>
            </div>
          </div>
          <div className="g-ifoot">
            <span className="note">The crew keeps working while you decide. Only the Verifier waits.</span>
            <button className="g-btn">Reject</button>
            <button className="g-btn g-btn--primary">Approve plan</button>
          </div>
        </section>
      </div>
    </ShellG>
  );
}
