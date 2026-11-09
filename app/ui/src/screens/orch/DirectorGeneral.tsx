// The project's Director-General (FG-02, ADR-15 §2): one conversation per project. You ask; it answers on its stream
// (`<tag>\t<text>` lines, see transcript.ts); it can propose runs, and only you start them (D4). It costs nothing at
// idle: one status read and one stream window when the view opens, then it follows `changed` for its own stream, and
// polls its status only while a turn is in flight.
//
// Every read and action goes through dgApi (FG-02c's project-keyed shapes, each action with its envelope). Until FG-02c
// lands the core addresses these by a run of the project (`runId`), so a project with no run yet cannot be asked; the
// view says so.
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { Badge, Button, Icon, Kbd, ariaShortcut } from "../../../components/ui/base";
import { S } from "../../../strings";
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
import { turns } from "./transcript";
import "./dg.css";

// The per-worker provider editor loads only for a proposal that carries workers (it brings the accounts and pickers with it).
const ProposalWorkers = lazy(() => import("./ProposalWorkers"));

const WINDOW_ROWS = 400;
const BUSY_POLL_MS = 2000;
const EVENT_MIN_GAP_MS = 500;

const sentence = (s: string) => (s ? s.slice(0, 1).toUpperCase() + s.slice(1) : s);

export function DirectorGeneral() {
  const { project, dgRun, proposals = [] } = useCurrentProject();
  const [st, setSt] = useState<LeadStatus | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [stream, setStream] = useState<string | null>(null);
  const [generation, setGeneration] = useState<number | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [echo, setEcho] = useState<string | null>(null);
  const [askId, setAskId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [askErr, setAskErr] = useState("");
  const [sending, setSending] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  const end = useRef<HTMLDivElement>(null);

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

  // The run (so the project) the view is on now. `still(run, at)` holds while the view is on that run and nothing
  // has switched since `at` (an epoch).
  const onRun = useRef(dgRun);
  onRun.current = dgRun;
  const still = (run: string | null | undefined, at: number) => onRun.current === run && epoch.current === at;
  const readStatus = useCallback(async () => {
    if (!dgRun) return;
    const n = ++statusReads.current.asked;
    const at = epoch.current;
    const current = () => onRun.current === dgRun && epoch.current === at && n > statusReads.current.applied;
    try {
      const s = await dg.status({ projectId: null, runId: dgRun });
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
  }, [dgRun, follow]);

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
  useEffect(() => { end.current?.scrollIntoView?.({ block: "end" }); }, [lines.length, showEcho, proposals.length]);

  const available = st?.available === true;
  const canAsk = available && !busy && !sending;
  // The core's id once it has answered with one (the T1 project id); the run serves the interim calls.
  const target: Target = { projectId: st?.project_id ?? null, runId: dgRun ?? null };

  const send = async () => {
    const text = draft.trim();
    if (!dgRun || !text || !canAsk) return;
    const run = dgRun, at = epoch.current;
    setSending(true);
    setAskErr("");
    const r = await dg.ask(target, envelope(target, actionId(), generation), text);
    setSending(false);
    if (!still(run, at)) return;
    if (!r.ok) { setAskErr(r.outcome.text); return; }
    setAskId(r.value.ask_id);
    setEcho(text);
    setDraft("");
    if (typeof r.value.generation === "number") setGeneration(r.value.generation);
    if (r.value.stream && r.value.stream !== active.current) follow(r.value.stream);
  };
  const cancel = async () => {
    if (!askId) return;
    const run = dgRun, at = epoch.current;
    const r = await dg.cancel(target, envelope(target, actionId(), null), askId);
    if (!still(run, at)) return;
    if (!r.ok) setAskErr(r.outcome.text);
    void readStatus();
  };
  const fresh = async () => {
    if (!dgRun || busy) return;
    const run = dgRun, at = epoch.current;
    const r = await dg.newConversation(target, envelope(target, actionId(), generation));
    if (!still(run, at)) return;
    if (!r.ok) { setAskErr(r.outcome.text); return; }
    setGeneration(r.value.generation);
    follow(r.value.stream);
    setLines([]);
    setEcho(null);
  };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void send(); }
  };

  useCommands(dgRun ? [
    { id: "dg-ask", label: S.dg.ask, run: () => field.current?.focus() },
    { id: "dg-new", label: S.dg.newConversation, run: () => void fresh() },
  ] : []);

  const launch = project ? toHash("launch", {}, project.id) : "#/launch";
  // No project yet (first run): there is no Director-General to talk to; the launcher is how a project begins.
  if (!project) return <Home />;
  const name = project?.name ?? "";

  if (!dgRun) {
    return (
      <section className="v-screen v-dgv" aria-label={S.dg.title(name)}>
        <div className="v-dgv__state">
          <Empty title={S.dg.noRun} action={<a className="v-dgv__link" href={launch}>{S.dg.startYourself}</a>}>
            {S.dg.noRunHint}
          </Empty>
        </div>
        <Proposals items={proposals} target={target} />
      </section>
    );
  }

  const stateKey = !st ? null : !available ? "unavailable" : busy ? "thinking" : st.state === "running" ? "running" : "idle";
  const stateLine = !st ? "" : !available ? (st.read_only_reason ?? S.dg.state.unavailable) : busy ? sentence(st.reason && st.busy ? st.reason : S.dg.thinking) : sentence(st.reason ?? "");

  return (
    <section className="v-screen v-dgv" aria-label={S.dg.title(name)}>
      <header className="v-dgv__head">
        <div className="v-dgv__who">
          <h1 className="v-dgv__name">{S.views.orch}</h1>
          {stateKey && <span className={`v-dgv__state-line v-dgv__state-line--${stateKey}`}><i aria-hidden="true" />{stateLine}</span>}
        </div>
        <div className="v-dgv__acts">
          {generation !== null && <span className="v-dgv__gen" title={S.dg.generationHint}>{S.dg.generation(generation)}</span>}
          <Button size="sm" variant="ghost" disabled={busy || !available} onClick={() => void fresh()}>{S.dg.newConversation}</Button>
          <a className="v-dgv__link" href={launch}>{S.dg.startYourself}</a>
        </div>
      </header>

      {loadErr && !st && <div className="v-dgv__state"><Problem title={S.dg.loadFailed} detail={loadErr} retry={() => void readStatus()} /></div>}

      <div className="v-dgv__scroll">
        <div className="v-dgv__col">
          <div role="log" aria-label={S.dg.transcript} aria-live="polite" className="v-dgv__log">
            {shown.length === 0 && !showEcho && st && (
              <div className="v-dgv__empty"><p className="v-dgv__empty-t">{S.dg.empty}</p><p>{S.dg.emptyHint}</p></div>
            )}
            {shown.map((t, i) => <TurnView key={i} role={t.role} text={t.text} />)}
            {showEcho && <TurnView role="you" text={echo!} />}
            {busy && <p className="v-dgv__thinking" role="status"><i aria-hidden="true" />{S.dg.thinking}</p>}
          </div>
          <Proposals items={proposals} target={target} />
          <div ref={end} />
        </div>
      </div>

      <form className="v-dgv__composer" onSubmit={(e) => { e.preventDefault(); void send(); }}>
        <div className="v-dgv__box">
          <label className="v-sr" htmlFor="dg-ask">{S.dg.ask}</label>
          <textarea id="dg-ask" ref={field} className="v-dgv__field" rows={2} value={draft} placeholder={S.dg.askPlaceholder}
            onChange={(e) => setDraft(e.target.value)} onKeyDown={onKey} readOnly={!available}
            aria-disabled={!available || undefined} aria-describedby={askErr ? "dg-ask-err" : undefined}
            aria-keyshortcuts={ariaShortcut(["⌘", "↵"])} />
          <div className="v-dgv__bar">
            {askErr ? <span id="dg-ask-err" className="v-dgv__err" role="alert">{askErr}</span> : <span className="v-dgv__hint"><Kbd keys={["⌘", "↵"]} decorative /></span>}
            {busy && askId
              ? <Button size="sm" variant="secondary" onClick={() => void cancel()}>{S.dg.cancel}</Button>
              : <Button size="sm" variant="primary" type="submit" disabled={!canAsk || !draft.trim()} busy={sending}>{S.dg.send}</Button>}
          </div>
        </div>
      </form>
    </section>
  );
}

function TurnView({ role, text }: { role: "you" | "dg" | "tool" | "error"; text: string }) {
  if (role === "tool") {
    return (
      <details className="v-turn v-turn--tool">
        <summary>{S.dg.tools}</summary>
        <p className="v-turn__t">{text}</p>
      </details>
    );
  }
  const who = role === "you" ? S.dg.you : role === "error" ? S.dg.problem : S.views.orch;
  return (
    <article className={`v-turn v-turn--${role}`}>
      <h2 className="v-turn__who">{role === "dg" && <Icon name="director" size={12} />}{who}</h2>
      <p className="v-turn__t">{text}</p>
    </article>
  );
}

function Proposals({ items, target }: { items: InboxItem[]; target: Target }) {
  if (items.length === 0) return null;
  return (
    <section className="v-props" aria-label={S.dg.proposals}>
      <p className="v-props__lede">{S.dg.proposalLede}</p>
      {items.map((p) => <ProposalCard key={p.id} item={p} target={target} />)}
    </section>
  );
}

function ProposalCard({ item, target }: { item: InboxItem; target: Target }) {
  const [out, setOut] = useState<Outcome | null>(null);
  // One action id per click. After an unknown outcome the same click is retried with the same id, so the core answers
  // it once (it may already have started); any settled answer retires the id.
  const ids = useRef<{ start?: string; dismiss?: string }>({});
  // The picks the user changed on the workers, to be carried by Start (nothing is sent before).
  const [overrides, setOverrides] = useState<ProposalOverride[]>([]);
  const hasWorkers = Array.isArray((item as { workers?: unknown }).workers) && ((item as { workers?: unknown[] }).workers?.length ?? 0) > 0;
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
