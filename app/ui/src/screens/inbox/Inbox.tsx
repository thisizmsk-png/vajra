// The Working Inbox (FG-07): every question, plan to approve and failure that waits on you, from every project, in one
// place. It reads the shell's one project view (`useProjects`, which already reloads on the store's change events with
// the 500 ms gap), so it adds no polling and no subscription. Opening an item only moves focus to its project and
// campaign; approving takes an explicit click on the exact plan hash shown, returns a receipt, and a stale plan fails.
import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, newActionId } from "../../ipc";
import type { ActResult, InboxItem } from "../../ipc";
import { UNSORTED_ID, actionProject, itemProject } from "../../projects/adapter";
import { attentionText } from "../../projects/attention";
import type { AttentionKind } from "../../projects/attention";
import { useProjects } from "../../projects/store";
import { useRoute } from "../../router";
import { Empty, Problem } from "../states";
import { escapeControls } from "../history/History";

const SNOOZE_MS = 15 * 60_000;
/** One blocker as the human saw it: a changed plan, or a different item, is a different key and cannot stay snoozed. */
const keyOf = (i: CampaignItem) => `${i.id}:${i.plan_sha256 ?? ""}`;
/** The FG-07 inbox lists the items of a campaign. A `run_proposal` names its project by root and has no campaign: the Director-General surface shows it, not this list. */
type CampaignItem = InboxItem & { campaign: string; kind: Exclude<InboxItem["kind"], "run_proposal"> };
const isCampaignItem = (i: InboxItem): i is CampaignItem => i.kind !== "run_proposal" && typeof i.campaign === "string";
const kindOf = (i: CampaignItem): AttentionKind => (i.kind === "task" && i.status === "failed" ? "failed" : i.kind);

type Outcome = { busy: boolean; text: string };

function receiptText(r: ActResult, done: string = S.inbox.receipt.done): string {
  const c = r.receipt;
  if (!c) return S.inbox.receipt.notDone;
  if (c.state === "done") return done;
  if (c.state === "accepted") return S.inbox.receipt.accepted;
  if (c.state === "unknown") return S.inbox.receipt.unknown;
  if (c.state === "failed") return S.inbox.receipt.failed;
  return c.reason === "plan_changed" || c.reason === "stale_generation" ? S.inbox.receipt.planChanged : S.inbox.receipt.refused(c.reason ?? "refused");
}

export function Inbox() {
  const P = useProjects();
  const inbox = useMemo(() => P.inbox.filter(isCampaignItem), [P.inbox]);
  const [, go] = useRoute();
  const [until, setUntil] = useState<Record<string, number>>({});
  const [showSnoozed, setShowSnoozed] = useState(false);
  const [outcome, setOutcome] = useState<Record<string, Outcome>>({});
  const [reasons, setReasons] = useState<Record<string, string>>({});
  const seen = useRef<Set<string> | null>(null);
  const [said, setSaid] = useState("");

  const index = useMemo(() => ({ projects: P.projects, byCampaign: P.byCampaign }), [P.projects, P.byCampaign]);
  // The core's ids decide where an item belongs and what an action names; the campaign's folder is only the fallback.
  const name = (i: CampaignItem) => P.projects.find((p) => p.id === itemProject(index, i))?.name ?? S.projects.unsorted;
  // Expiry must cause a render by itself: nothing else re-renders an idle inbox. One timer, for the earliest snooze.
  const [ticks, tick] = useState(0);
  useEffect(() => {
    const next = Math.min(...Object.values(until).filter((t) => t > Date.now()));
    if (!Number.isFinite(next)) return;
    const h = setTimeout(() => tick((n) => n + 1), Math.max(0, next - Date.now()) + 50);
    return () => clearTimeout(h);
  }, [until, ticks]);
  const now = Date.now();
  const snoozed = (i: CampaignItem) => (until[keyOf(i)] ?? 0) > now;
  const live = inbox.filter((i) => !snoozed(i));
  const later = inbox.filter(snoozed);

  // Announce what newly arrived, by kind and project only (never the title or body: untrusted, maybe secret).
  const keys = inbox.map(keyOf).join("|");
  const announce = useMemo(() => {
    if (P.state !== "ok") return null;
    const before = seen.current;
    seen.current = new Set(inbox.map(keyOf));
    if (before === null) return null;
    const fresh = inbox.filter((i) => !before.has(keyOf(i)));
    const last = fresh[fresh.length - 1];
    return last ? attentionText(kindOf(last), name(last)) : null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keys, P.state]);
  if (announce && announce !== said) setSaid(announce);

  const open = (i: CampaignItem) => {
    const project = itemProject(index, i) ?? null;
    // The link pins this project: the shell would otherwise place the campaign by its own folder.
    const pin: Record<string, string> = project ? { pin: "1" } : {};
    if (i.kind !== "plan" && P.crew && P.crew.campaign === i.campaign) go("crew", { slug: i.campaign, id: P.crew.runId, ...pin }, project);
    else go("review", { slug: i.campaign, ...pin }, project);
  };
  const set = (k: string, o: Outcome) => setOutcome((m) => ({ ...m, [k]: o }));
  const approve = (i: CampaignItem) => {
    const k = keyOf(i);
    const project = actionProject(index, i);
    // No id at all (an older core, a campaign no folder places): nothing is sent (the card says why).
    if (outcome[k]?.busy || !i.plan_sha256 || !project) return;
    set(k, { busy: true, text: "" });
    const envelope = { action_id: newActionId(), project_id: project, expect_generation: null };
    void api.actApprovePlan(envelope, i.id, i.plan_sha256).then(
      (r) => { set(k, { busy: false, text: receiptText(r) }); P.reload(); },
      () => { set(k, { busy: false, text: S.inbox.receipt.notDone }); },
    );
  };
  const reject = (i: CampaignItem) => {
    const k = keyOf(i);
    const reason = (reasons[k] ?? "").trim();
    // The hash the human read is required: a reject without it could land on whichever plan is current, unseen.
    if (outcome[k]?.busy || !i.plan_sha256 || reason.length < 1 || reason.length > 500) return;
    set(k, { busy: true, text: "" });
    const envelope = { action_id: newActionId(), project_id: actionProject(index, i) ?? UNSORTED_ID, expect_generation: null };
    void api.actRejectPlan(envelope, i.id, reason, i.plan_sha256).then(
      (r) => { set(k, { busy: false, text: receiptText(r, S.inbox.receipt.rejected) }); P.reload(); },
      () => { set(k, { busy: false, text: S.inbox.receipt.notDone }); },
    );
  };

  const groups = P.projects.map((p) => ({ p, items: live.filter((i) => itemProject(index, i) === p.id) })).filter((g) => g.items.length > 0);
  // An item whose campaign no project owns yet (a run that just started, or a list that lagged the inbox) still shows:
  // under Unsorted, never dropped. Nothing waiting on you can be hidden by a missing project.
  const placed = new Set(groups.flatMap((g) => g.items));
  const loose = live.filter((i) => !placed.has(i));
  const unsortedGroup = groups.find((g) => g.p.bucket === "unsorted");

  const card = (i: CampaignItem, isSnoozed: boolean) => {
    const k = keyOf(i);
    const o = outcome[k];
    const reason = reasons[k] ?? "";
    const kind = kindOf(i);
    return (
      <article key={k} className="v-ibx" aria-label={i.title}>
        <div className="v-ibx__top">
          <Badge status={kind === "failed" ? "danger" : "warning"}>{S.inbox.kinds[kind]}</Badge>
          <h3 className="v-ibx__t">{i.title}</h3>
        </div>
        {i.kind === "plan" && (
          <div className="v-ibx__plan">
            <p className="v-ibx__warn">{S.inbox.unsigned}</p>
            {i.plan && (
              <>
                <p>{i.plan.target} · {S.inbox.risk(i.plan.risk)}</p>
                <ul aria-label={S.inbox.commands}>{i.plan.commands.map((c) => <li key={c}><code>{c}</code></li>)}</ul>
                {i.plan.rollback.length > 0 && <ul aria-label={S.inbox.rollback}>{i.plan.rollback.map((c) => <li key={c}><code>{c}</code></li>)}</ul>}
                {i.plan.health_checks.length > 0 && <ul aria-label={S.inbox.healthChecks}>{i.plan.health_checks.map((c) => <li key={c}><code>{c}</code></li>)}</ul>}
              </>
            )}
            {typeof i.action === "string" && i.action && <p>{S.inbox.action(i.action)}</p>}
            {i.body && <p className="v-ibx__body">{i.body}</p>}
            {i.entry && <pre className="v-ibx__entry" aria-label={S.inbox.fullEntry} tabIndex={0}>{escapeControls(JSON.stringify(i.entry, null, 2))}</pre>}
            {i.plan_sha256 && <p>{S.inbox.planHash}: <code className="v-ibx__hash">{i.plan_sha256}</code></p>}
            {i.invalid && <p role="note">{i.invalid}</p>}
          </div>
        )}
        {i.kind !== "plan" && i.body && <p className="v-ibx__body">{i.body}</p>}
        <div className="v-ibx__acts">
          <Button size="sm" variant="ghost" onClick={() => open(i)}>{S.inbox.open}</Button>
          {i.kind === "plan" && !i.invalid && i.plan_sha256 && (
            <Button size="sm" disabled={o?.busy || !actionProject(index, i)} onClick={() => approve(i)}>{S.inbox.approve}</Button>
          )}
          {i.kind === "plan" && !i.invalid && i.plan_sha256 && !actionProject(index, i) && <p className="v-ibx__warn" role="note">{S.inbox.needsProject}</p>}
          {isSnoozed
            ? <Button size="sm" variant="ghost" onClick={() => setUntil((u) => { const { [k]: _, ...rest } = u; return rest; })}>{S.inbox.unsnooze}</Button>
            : <Button size="sm" variant="ghost" onClick={() => setUntil((u) => ({ ...u, [k]: Date.now() + SNOOZE_MS }))}>{S.inbox.snooze}</Button>}
        </div>
        {i.kind === "plan" && (
          <div className="v-ibx__rej">
            <input type="text" className="v-input" placeholder={S.inbox.reason} aria-label={S.inbox.reason} value={reason} onChange={(e) => setReasons((m) => ({ ...m, [k]: e.target.value }))} />
            <Button size="sm" variant="ghost" disabled={o?.busy || !i.plan_sha256 || reason.trim().length < 1 || reason.trim().length > 500} onClick={() => reject(i)}>{S.inbox.reject}</Button>
          </div>
        )}
        {o?.text && <p role="status" className="v-ibx__out">{o.text}</p>}
      </article>
    );
  };

  return (
    <section className="v-screen v-inbox" aria-label={S.inbox.title}>
      <div className="v-board__bar">
        <span className="v-board__note">{S.inbox.title}</span>
        <Button size="sm" variant="ghost" onClick={P.reload}>{S.inbox.refresh}</Button>
      </div>
      <p role="status" className="v-sr">{said}</p>
      {P.state === "ok" && !P.inboxFailed && live.length === 0 && later.length === 0 && <div className="v-board__state"><Empty title={S.inbox.empty}>{S.inbox.emptyHint}</Empty></div>}
      {(P.state === "error" || P.inboxFailed) && <div className="v-board__state"><Problem title={S.inbox.failed} detail={P.error} retry={P.reload} /></div>}
      {groups.map(({ p, items }) => (
        <section key={p.id} className="v-inbox__group" aria-label={p.name}>
          <h2 className="v-inbox__h">{p.name}</h2>
          {items.map((i) => card(i, false))}
          {p === unsortedGroup?.p && loose.map((i) => card(i, false))}
        </section>
      ))}
      {loose.length > 0 && !unsortedGroup && (
        <section className="v-inbox__group" aria-label={S.projects.unsorted}>
          <h2 className="v-inbox__h">{S.projects.unsorted}</h2>
          {loose.map((i) => card(i, false))}
        </section>
      )}
      {later.length > 0 && (
        <section className="v-inbox__group">
          <button type="button" className="v-inbox__more" aria-expanded={showSnoozed} onClick={() => setShowSnoozed((x) => !x)}>{S.inbox.snoozed(later.length)}</button>
          {showSnoozed && later.map((i) => card(i, true))}
        </section>
      )}
    </section>
  );
}
