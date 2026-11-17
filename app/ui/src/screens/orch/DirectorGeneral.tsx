// The project's Director-General (FG-02, ADR-15 §2): one conversation per project. You ask; it answers on its stream
// (`<tag>\t<text>` lines, see transcript.ts); it can propose runs, and only you start them (D4). It costs nothing at
// idle: one status read and one stream window when the view opens, then it follows `changed` for its own stream, and
// polls its status only while a turn is in flight.
//
// Every read and action goes through dgApi (FG-02c's project-keyed shapes, each action with its envelope). M6: the
// Director-General is the project's lead chat agent before any run exists, so a registered project is asked by its id,
// run or none. Only a folder the core has no project for yet needs a run of it; with neither, the view links to the
// launcher.
import { lazy, memo, Suspense, useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode, UIEvent } from "react";
import { Badge, Button, Icon, StatusDot } from "../../../components/ui/base";
import { S } from "../../../strings";
import type { StatusKey } from "../../../strings";
import { errorText } from "../../ipc";
import type { InboxItem, LeadStatus } from "../../ipc";
import { useCurrentProject } from "../../projects/current";
import { useCommands } from "../../palette";
import { toHash } from "../../router";
import { Empty, LoadingRows, Problem } from "../states";
import type { ProposalOverride } from "../../accountsContract";
import { Home } from "../home/Home";
import { actionId, dg, envelope, useDgReload } from "./dgApi";
import type { Outcome, Target } from "./dgApi";
import { toolCalls, turns } from "./transcript";
import { Composer } from "./Composer";
import "./dg.css";

// The per-worker provider editor loads only for a proposal that carries workers (it brings the accounts and pickers with it).
const ProposalWorkers = lazy(() => import("./ProposalWorkers"));
/** How far from the end still counts as "at the end": a line of text, so a pixel of rounding never unpins the view. */
const END_SLACK_PX = 24;

const WINDOW_ROWS = 400;
const BUSY_POLL_MS = 2000;
const EVENT_MIN_GAP_MS = 500;

const sentence = (s: string) => (s ? s.slice(0, 1).toUpperCase() + s.slice(1) : s);

/**
 * `embedded`: the same conversation shown as a run's Lead on the Workers screen. It has the run's name over it, its
 * workers as cards that open their chats, and no proposals or palette commands (the Director-General view has those).
 */
export type Embedded = { title: string; status?: ReactNode; agents: SubAgents; banner?: ReactNode; intro?: ReactNode; /** The run on screen, asked by its id when its folder is not a registered project. */ runId?: string };

export function DirectorGeneral({ embedded }: { embedded?: Embedded } = {}) {
  const ctx = useCurrentProject();
  const { project, dgRun } = ctx;
  const proposals = embedded ? [] : ctx.proposals ?? [];
  // Who the view talks to (M6): a registered project by its core id, whether or not it has a run; else a run of the folder.
  const coreProject = project?.registered ? project.id : null;
  const runOnly = coreProject ? null : embedded?.runId ?? dgRun ?? null;
  // One key for "the conversation this view is on": a switch of project or run resets the view.
  const dgKey = coreProject ? `p:${coreProject}` : runOnly ? `r:${runOnly}` : null;
  const [st, setSt] = useState<LeadStatus | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [stream, setStream] = useState<string | null>(null);
  const [generation, setGeneration] = useState<number | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [echo, setEcho] = useState<string | null>(null);
  const [askId, setAskId] = useState<string | null>(null);
  const [askErr, setAskErr] = useState("");
  const [sending, setSending] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const pane = useRef<HTMLDivElement>(null);
  const frame = useRef(0);
  const watch = useRef<ResizeObserver | null>(null);
  const atEnd = useRef(true);
  const lastTop = useRef(0);

  // What the view is showing, so nothing that was asked earlier can bring an older state back:
  // - `active` is the stream the transcript belongs to;
  // - `epoch` changes on every switch (project or run, New conversation, an ask that moved streams), and an action's
  //   answer applies only if no switch happened while it was in flight;
  // - each window and status read is numbered, and applies only if no later one has, so overlapping poll, event and
  //   Cancel reads never go backwards.
  // `follow` is the one way to switch: it moves `active` at once and voids every read still in flight.
  const active = useRef<string | null>(null);
  const epoch = useRef(0);
  const reads = useRef({ asked: 0, applied: 0 });
  const statusReads = useRef({ asked: 0, applied: 0 });
  const follow = useCallback((s: string | null) => {
    active.current = s;
    epoch.current += 1;
    reads.current.applied = reads.current.asked;
    statusReads.current.applied = statusReads.current.asked;
    // Another conversation opens at its end, wherever the reader was in the last one.
    atEnd.current = true;
    lastTop.current = 0;
    setStream(s);
  }, []);

  const readStream = useCallback(async (s: string) => {
    const n = ++reads.current.asked;
    try {
      const w = await dg.window(s, WINDOW_ROWS);
      if (s !== active.current || n <= reads.current.applied) return;
      reads.current.applied = n;
      setLines(Array.isArray(w?.lines) ? w.lines.map((l) => l.text) : []);
    } catch {
      /* the status line already says whether it can be reached; an unreadable window leaves the last one */
    }
  }, []);

  // The conversation (project, or run of an unregistered folder) the view is on now. `still(key, at)` holds while the
  // view is on it and nothing has switched since `at` (an epoch).
  const onKey = useRef(dgKey);
  onKey.current = dgKey;
  const still = (key: string | null, at: number) => onKey.current === key && epoch.current === at;
  const readStatus = useCallback(async () => {
    if (!dgKey) return;
    const n = ++statusReads.current.asked;
    const at = epoch.current;
    const current = () => onKey.current === dgKey && epoch.current === at && n > statusReads.current.applied;
    try {
      const s = await dg.status({ projectId: coreProject, runId: runOnly });
      if (!current()) return;
      statusReads.current.applied = n;
      setSt(s);
      setLoadErr("");
      if (!s.busy) setAskId(null);
      else if (s.ask_id) setAskId(s.ask_id);
      if (typeof s.generation === "number") setGeneration(s.generation);
      if (s.stream && active.current === null) follow(s.stream);
    } catch (e) {
      if (current()) setLoadErr(errorText(e));
    }
  }, [dgKey, coreProject, runOnly, follow]);

  useEffect(() => {
    setSt(null); follow(null); setGeneration(null); setLines([]); setEcho(null); setAskId(null); setAskErr("");
    void readStatus();
  }, [readStatus, follow]);
  useEffect(() => { if (stream) void readStream(stream); }, [stream, readStream]);

  // Follow its own stream only; anything else moving is not this view's business.
  useDgReload(stream, () => { if (stream) void readStream(stream); }, EVENT_MIN_GAP_MS);

  const busy = !!askId || !!st?.busy;
  // While a turn runs, its status is the only way to learn it ended; nothing polls at idle.
  useEffect(() => {
    if (!busy) return;
    const t = setInterval(() => { void readStatus(); if (stream) void readStream(stream); }, BUSY_POLL_MS);
    return () => clearInterval(t);
  }, [busy, readStatus, readStream, stream]);

  const shown = turns(lines);
  const lastYou = [...shown].reverse().find((t) => t.role === "you");
  const showEcho = echo !== null && lastYou?.text !== echo;
  // New lines, the reader's own question and a new proposal bring the end into view, but only while the reader is at
  // the end. Someone who scrolled up to read is left where they are; asking a question or scrolling back down pins
  // them again.
  // The scroll is one write of `scrollTop` in the next animation frame, however many things ask for it in between: a batch
  // of lines, the resize it causes and a new proposal share a single forced layout instead of one each (and no
  // `scrollIntoView`, which also walks every scrollable ancestor).
  const pin = useCallback(() => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      const el = pane.current;
      if (el && atEnd.current) el.scrollTop = el.scrollHeight;
    });
  }, []);
  useEffect(() => () => { if (frame.current) cancelAnimationFrame(frame.current); frame.current = 0; }, []);
  useEffect(() => { if (atEnd.current) pin(); }, [lines.length, showEcho, proposals.length, pin]);
  // A proposal's worker table and a long answer arrive after that scroll and push the newest turn (and Start run) below
  // the fold. While the reader is at the end, growth keeps them there; once they scroll up, nothing moves them.
  // A callback ref, not a mount effect: the column only exists once there is a project with a run, which can be long
  // after this view first renders. Each column that appears is watched, and starts pinned to its end.
  const col = useCallback((el: HTMLDivElement | null) => {
    watch.current?.disconnect();
    watch.current = null;
    if (!el || typeof ResizeObserver === "undefined") return;
    atEnd.current = true;
    lastTop.current = 0;
    const ro = new ResizeObserver(() => { if (atEnd.current) pin(); });
    ro.observe(el);
    watch.current = ro;
  }, [pin]);
  // Growth never moves the scroll position up, so only an upward scroll means the reader left the end. (Judging every
  // scroll event by its distance from the end would unpin on our own scroll when more content lands before the event.)
  const onScroll = (e: UIEvent<HTMLDivElement>) => {
    const s = e.currentTarget;
    if (s.scrollHeight - s.scrollTop - s.clientHeight <= END_SLACK_PX) atEnd.current = true;
    else if (s.scrollTop < lastTop.current) atEnd.current = false;
    lastTop.current = s.scrollTop;
  };

  const available = st?.available === true;
  const canAsk = available && !busy && !sending;
  // A registered project's core id; for an unregistered folder, the core's id once it has answered with one, and the run.
  const target: Target = { projectId: coreProject ?? (runOnly ? st?.project_id ?? null : null), runId: runOnly };
  // Commands go to the project when it is registered, else to the run: never the core's id for a run-only folder, which
  // the status read has not proven is this view's.
  const callTarget: Target = { projectId: coreProject, runId: runOnly };

  // Optimistic: the question shows at once (the composer clears its own draft) and the answer only confirms it. A refusal
  // takes the echo back and returns false, so the composer puts the text back. Returns whether the ask was taken.
  const send = async (raw: string): Promise<boolean> => {
    const text = raw.trim();
    if (!dgKey || !text || !canAsk) return false;
    const key = dgKey, at = epoch.current;
    setSending(true);
    setAskErr("");
    atEnd.current = true;
    setEcho(text);
    const r = await dg.ask(callTarget, envelope(target, actionId(), generation), text);
    setSending(false);
    if (!still(key, at)) return false;
    if (!r.ok) { setEcho(null); setAskErr(r.outcome.text); return false; }
    setAskId(r.value.ask_id);
    if (typeof r.value.generation === "number") setGeneration(r.value.generation);
    if (r.value.stream && r.value.stream !== active.current) follow(r.value.stream);
    return true;
  };
  // The composer is memoised and must not see a new function on every stream render, so it calls the latest `send`
  // and `cancel` through these.
  const sendRef = useRef(send);
  sendRef.current = send;
  const onSend = useCallback((text: string) => sendRef.current(text), []);
  const cancel = async () => {
    if (!askId) return;
    const key = dgKey, at = epoch.current;
    const r = await dg.cancel(callTarget, envelope(target, actionId(), null), askId);
    if (!still(key, at)) return;
    if (!r.ok) setAskErr(r.outcome.text);
    void readStatus();
  };
  const fresh = async () => {
    if (!dgKey || busy) return;
    const key = dgKey, at = epoch.current;
    const r = await dg.newConversation(callTarget, envelope(target, actionId(), generation));
    if (!still(key, at)) return;
    if (!r.ok) { setAskErr(r.outcome.text); return; }
    setGeneration(r.value.generation);
    follow(r.value.stream);
    setLines([]);
    setEcho(null);
  };
  const cancelRef = useRef(cancel);
  cancelRef.current = cancel;
  const onCancel = useCallback(() => void cancelRef.current(), []);

  useCommands(dgKey && !embedded ? [
    { id: "dg-ask", label: S.dg.ask, run: () => field.current?.focus() },
    { id: "dg-new", label: S.dg.newConversation, run: () => void fresh() },
  ] : []);

  const launch = project ? toHash("launch", {}, project.id) : "#/launch";
  // No project yet (first run): there is no Director-General to talk to; the launcher is how a project begins.
  if (!project && !embedded) return <Home />;
  const name = project?.name ?? "";

  // A folder the core has no project for, and no run of it: nothing to address yet. A registered project never lands
  // here (M6), run or none.
  if (!dgKey) {
    return (
      <section className="v-screen v-dgv" aria-label={S.dg.title(name)}>
        <div className="v-dgv__state">
          <Empty title={S.dg.noProject} action={<a className="v-dgv__link" href={launch}>{S.dg.startYourself}</a>}>
            {S.dg.noProjectHint}
          </Empty>
        </div>
        <Proposals items={proposals} target={target} />
      </section>
    );
  }

  const stateKey = !st ? null : !available ? "unavailable" : busy ? "thinking" : st.state === "running" ? "running" : "idle";
  const stateLine = !st ? "" : !available ? (st.read_only_reason ?? S.dg.state.unavailable) : busy ? sentence(st.reason && st.busy ? st.reason : S.dg.thinking) : sentence(st.reason ?? "");

  const agents = embedded?.agents;
  return (
    <section className={embedded ? "v-crew__pane v-dgv v-dgv--embedded" : "v-screen v-dgv"} aria-label={embedded ? embedded.title : S.dg.title(name)}>
      <header className="v-dgv__head">
        <div className="v-dgv__who">
          {embedded ? <h2 className="v-dgv__name">{embedded.title}</h2> : <h1 className="v-dgv__name">{S.views.orch}</h1>}
          {embedded?.status}
          {stateKey && (!embedded || stateKey === "thinking" || stateKey === "unavailable") && <span className={`v-dgv__state-line v-dgv__state-line--${stateKey}`}><i aria-hidden="true" />{stateLine}</span>}
        </div>
        <div className="v-dgv__acts">
          {generation !== null && <span className="v-dgv__gen" title={S.dg.generationHint}>{S.dg.generation(generation)}</span>}
          <Button size="sm" variant="ghost" disabled={busy || !available} onClick={() => void fresh()}>{S.dg.newConversation}</Button>
          {!embedded && <a className="v-dgv__link" href={launch}>{S.dg.startYourself}</a>}
        </div>
      </header>
      {embedded?.banner}

      {loadErr && !st && <div className="v-dgv__state"><Problem title={S.dg.loadFailed} detail={loadErr} retry={() => void readStatus()} /></div>}

      <div className="v-dgv__scroll" ref={pane} onScroll={onScroll}>
        <div className="v-dgv__col" ref={col}>
          {embedded && <p className="v-subagents__lede">{S.dg.leadIs}</p>}
          {embedded?.intro}
          {agents && agents.list.length > 0 && (
            <section className="v-subagents" aria-label={S.dg.startedWorkers(agents.list.length)}>
              <p className="v-subagents__lede">{S.dg.startedWorkers(agents.list.length)}</p>
              <ul className="v-subagents__list">
                {agents.list.map((a) => <li key={a.id}><SubAgentCard agent={a} verb={S.dg.workerVerb.start} onOpen={() => agents.open(a.id)} /></li>)}
              </ul>
            </section>
          )}
          <div role="log" aria-label={S.dg.transcript} aria-live="polite" className="v-dgv__log">
            {shown.length === 0 && !showEcho && st && (
              <div className="v-dgv__empty"><p className="v-dgv__empty-t">{S.dg.empty}</p><p>{embedded ? S.dg.emptyHintLead : S.dg.emptyHint}</p></div>
            )}
            {shown.map((t, i) => <TurnView key={i} role={t.role} text={t.text} agents={agents} />)}
            {showEcho && <TurnView role="you" text={echo!} />}
            {busy && <p className="v-dgv__thinking" role="status"><i aria-hidden="true" />{S.dg.thinking}</p>}
          </div>
          <Proposals items={proposals} target={target} />
          <div aria-hidden="true" />
        </div>
      </div>

      <Composer id={embedded ? "lead-ask" : "dg-ask"} label={S.dg.ask} placeholder={embedded ? S.dg.askLeadPlaceholder : S.dg.askPlaceholder}
        field={field} available={available} canAsk={canAsk} cancellable={busy && !!askId} sending={sending} askErr={askErr} onSend={onSend} onCancel={onCancel} />
    </section>
  );
}

/** A sub-agent the Lead can link to: its readable name, its status and how to open its chat. */
export type SubAgent = { id: string; name: string; task: string | null; status: StatusKey; statusLabel: string; last?: string };
/** What an embedded Lead chat knows about the run's workers. */
export type SubAgents = { list: SubAgent[]; open: (id: string) => void };

/** A link to one worker's chat: who, what it is on, its status and, while it works, its newest line. */
export function SubAgentCard({ agent, verb, text, onOpen, href }: { agent: SubAgent | null; verb: string; text?: string; onOpen?: () => void; href?: string }) {
  const body = (
    <>
      <span className="v-subagent__top">
        <span className="v-subagent__verb">{verb}</span>
        <span className="v-subagent__name">{agent?.name ?? ""}</span>
        {agent && <StatusDot status={agent.status} label={agent.statusLabel} className="v-subagent__dot" />}
      </span>
      {agent?.task && <span className="v-subagent__task">{agent.task}</span>}
      {text && <span className="v-subagent__text">{text}</span>}
      {!text && agent?.last && <span className="v-subagent__text">{agent.last}</span>}
      <span className="v-subagent__open">{S.dg.openChat}</span>
    </>
  );
  return onOpen
    ? <button type="button" className="v-subagent" onClick={onOpen}>{body}</button>
    : <a className="v-subagent" href={href}>{body}</a>;
}

const TurnView = memo(function TurnView({ role, text, agents }: { role: "you" | "dg" | "tool" | "error"; text: string; agents?: SubAgents }) {
  if (role === "tool") {
    // A call that messaged, redirected, inspected or stopped a worker is a card that opens that worker's chat; the rest
    // fold away as before.
    const calls = toolCalls(text);
    const rest = calls.filter((c) => !c.worker).map((c) => c.line);
    return (
      <>
        {calls.filter((c) => c.worker).map((c, i) => {
          const a = agents?.list.find((w) => w.id === c.worker || w.id === `agent:${c.worker}`) ?? null;
          const shown = a ?? { id: c.worker!, name: c.worker!.replace(/^agent:/, ""), task: null, status: "unknown" as StatusKey, statusLabel: "" };
          const open = a && agents ? () => agents.open(a.id) : undefined;
          return <SubAgentCard key={i} agent={a ? a : { ...shown, statusLabel: S.status.unknown }} verb={S.dg.workerVerb[c.action]} text={c.text ?? undefined}
            onOpen={open} href={open ? undefined : toHash("crew", { worker: shown.id.replace(/^agent:/, "") })} />;
        })}
        {rest.length > 0 && (
          <details className="v-turn v-turn--tool">
            <summary>{S.dg.tools}</summary>
            <p className="v-turn__t">{rest.join("\n")}</p>
          </details>
        )}
      </>
    );
  }
  const who = role === "you" ? S.dg.you : role === "error" ? S.dg.problem : S.views.orch;
  return (
    <article className={`v-turn v-turn--${role}`}>
      <h2 className="v-turn__who">{role === "dg" && <Icon name="director" size={12} />}{who}</h2>
      <p className="v-turn__t">{text}</p>
    </article>
  );
});

function Proposals({ items, target }: { items: InboxItem[]; target: Target }) {
  if (items.length === 0) return null;
  return (
    <section className="v-props" aria-label={S.dg.proposals}>
      <p className="v-props__lede">{S.dg.proposalLede}</p>
      {items.map((p) => <ProposalCard key={p.id} item={p} target={target} />)}
    </section>
  );
}

/** The plan's threads at a glance, by owner: "4 threads: 2 Backend, 1 Docs, 1 Tests". The table below has each one's
 *  model, effort and why. A light read of the raw list, so the pickers' chunk still loads only when there are workers. */
function threadLine(workers: unknown): string | null {
  if (!Array.isArray(workers) || workers.length === 0) return null;
  const by = new Map<string, number>();
  for (const w of workers.slice(0, 16)) {
    const role = (w as { role?: unknown } | null)?.role;
    if (typeof role !== "string" || !role) continue;
    const label = S.agents.role[role] ?? role;
    by.set(label, (by.get(label) ?? 0) + 1);
  }
  const n = [...by.values()].reduce((a, b) => a + b, 0);
  return n === 0 ? null : S.dg.threadCount(n, [...by].map(([r, c]) => `${c} ${r}`).join(", "));
}

function ProposalCard({ item, target }: { item: InboxItem; target: Target }) {
  const [out, setOut] = useState<Outcome | null>(null);
  // One action id per click. After an unknown outcome the same click is retried with the same id, so the core answers
  // it once (it may already have started); any settled answer retires the id.
  const ids = useRef<{ start?: string; dismiss?: string }>({});
  // The picks the user changed on the workers, to be carried by Start (nothing is sent before).
  const [overrides, setOverrides] = useState<ProposalOverride[]>([]);
  const hasWorkers = Array.isArray((item as { workers?: unknown }).workers) && ((item as { workers?: unknown[] }).workers?.length ?? 0) > 0;
  const threads = threadLine((item as { workers?: unknown }).workers);
  const pending = out?.kind === "pending";
  const settled = out?.kind === "done";
  const act = async (what: "start" | "dismiss") => {
    if (pending || settled) return;
    setOut({ kind: "pending", text: S.dg.receipt.pending });
    const id = (ids.current[what] ??= actionId());
    // Start is checked against the project's generation (FG-02c: project_state.generations.project, not the item's
    // display generation); the interim core takes no envelope, so it is null until that read exists. Dismiss is
    // idempotent and expects nothing.
    const env = envelope(target, id, null);
    const r = what === "start" ? await dg.startProposal(target, env, item, overrides) : await dg.dismissProposal(target, env, item);
    if (r.kind !== "unknown") ids.current[what] = undefined;
    setOut(r);
  };
  return (
    <article className="v-prop" aria-label={S.dg.proposal}>
      <header className="v-prop__head">
        <h2 className="v-prop__t">{item.title}</h2>
        {!settled && <Badge status="warning">{S.board.needsYou}</Badge>}
      </header>
      <p className="v-prop__label">{S.dg.exactAsk}</p>
      <p className="v-prop__ask">{item.ask ?? item.body}</p>
      <dl className="v-prop__kv">
        {threads && <><dt>{S.dg.threads}</dt><dd>{threads}</dd></>}
        {item.base && <><dt>{S.dg.base}</dt><dd>{item.base}</dd></>}
        {typeof item.rounds === "number" && <><dt>{S.dg.rounds}</dt><dd>{item.rounds}</dd></>}
        {typeof item.generation === "number" && <><dt>{S.views.orch}</dt><dd>{S.dg.fromGeneration(item.generation)}</dd></>}
      </dl>
      {hasWorkers && (
        <Suspense fallback={<LoadingRows rows={2} label={S.states.loading} />}>
          <ProposalWorkers item={item} onOverrides={setOverrides} />
        </Suspense>
      )}
      <footer className="v-prop__foot">
        <p role="status" className={out ? `v-prop__out v-prop__out--${out.kind}` : "v-prop__out"}>{out?.text ?? ""}</p>
        <Button size="sm" variant="ghost" disabled={pending || settled} onClick={() => void act("dismiss")}>{S.dg.dismiss}</Button>
        <Button size="sm" variant="primary" disabled={pending || settled} onClick={() => void act("start")}>{S.dg.start}</Button>
      </footer>
    </article>
  );
}
