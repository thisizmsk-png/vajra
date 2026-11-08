// The Working Inbox (FG-07): every question, plan to approve and failure that waits on you, from every project, in one
// place. It reads the shell's one project view (`useProjects`, which already reloads on the store's change events with
// the 500 ms gap), so it adds no polling and no subscription. Opening an item only moves focus to its project and
// campaign; approving takes an explicit click on the exact plan hash shown, returns a receipt, and a stale plan fails.
import { useMemo, useRef, useState } from "react";
import { Badge, Button } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, newActionId } from "../../ipc";
import type { ActResult, InboxItem } from "../../ipc";
import { attentionText } from "../../projects/attention";
import type { AttentionKind } from "../../projects/attention";
import { useProjects } from "../../projects/store";
import { useRoute } from "../../router";
import { Empty } from "../states";

const SNOOZE_MS = 15 * 60_000;
/** One blocker as the human saw it: a changed plan, or a different item, is a different key and cannot stay snoozed. */
const keyOf = (i: InboxItem) => `${i.id}:${i.plan_sha256 ?? ""}`;
const kindOf = (i: InboxItem): AttentionKind => (i.kind === "task" && i.status === "failed" ? "failed" : i.kind);

type Outcome = { busy: boolean; text: string };

function receiptText(r: ActResult): string {
  const c = r.receipt;
  if (!c) return S.inbox.receipt.notDone;
  if (c.state === "done") return S.inbox.receipt.done;
  if (c.state === "accepted") return S.inbox.receipt.accepted;
  if (c.state === "unknown") return S.inbox.receipt.unknown;
  if (c.state === "failed") return S.inbox.receipt.failed;
  return c.reason === "plan_changed" || c.reason === "stale_generation" ? S.inbox.receipt.planChanged : S.inbox.receipt.refused(c.reason ?? "refused");
}

export function Inbox() {
  const P = useProjects();
  const [, go] = useRoute();
  const [until, setUntil] = useState<Record<string, number>>({});
  const [showSnoozed, setShowSnoozed] = useState(false);
  const [outcome, setOutcome] = useState<Record<string, Outcome>>({});
  const [reasons, setReasons] = useState<Record<string, string>>({});
  const seen = useRef<Set<string> | null>(null);
  const [said, setSaid] = useState("");

  const name = (campaign: string) => P.projects.find((p) => p.id === P.byCampaign[campaign])?.name ?? S.projects.unsorted;
  const now = Date.now();
  const snoozed = (i: InboxItem) => (until[keyOf(i)] ?? 0) > now;
  const live = P.inbox.filter((i) => !snoozed(i));
  const later = P.inbox.filter(snoozed);

  // Announce what newly arrived, by kind and project only (never the title or body: untrusted, maybe secret).
  const keys = P.inbox.map(keyOf).join("|");
  const announce = useMemo(() => {
    if (P.state !== "ok") return null;
    const before = seen.current;
    seen.current = new Set(P.inbox.map(keyOf));
    if (before === null) return null;
    const fresh = P.inbox.filter((i) => !before.has(keyOf(i)));
    const last = fresh[fresh.length - 1];
    return last ? attentionText(kindOf(last), name(last.campaign)) : null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keys, P.state]);
  if (announce && announce !== said) setSaid(announce);

  const open = (i: InboxItem) => {
    const project = P.byCampaign[i.campaign] ?? null;
    if (i.kind !== "plan" && P.crew && P.crew.campaign === i.campaign) go("crew", { slug: i.campaign, id: P.crew.runId }, project);
    else go("review", { slug: i.campaign }, project);
  };
  const set = (k: string, o: Outcome) => setOutcome((m) => ({ ...m, [k]: o }));
  const approve = (i: InboxItem) => {
    const k = keyOf(i);
    if (outcome[k]?.busy || !i.plan_sha256) return;
    set(k, { busy: true, text: "" });
    const envelope = { action_id: newActionId(), project_id: P.byCampaign[i.campaign] ?? "", expect_generation: null };
    void api.actApprovePlan(envelope, i.id, i.plan_sha256).then(
      (r) => { set(k, { busy: false, text: receiptText(r) }); P.reload(); },
      () => { set(k, { busy: false, text: S.inbox.receipt.notDone }); },
    );
  };
  const reject = (i: InboxItem) => {
    const k = keyOf(i);
    const reason = (reasons[k] ?? "").trim();
    if (outcome[k]?.busy || reason.length < 1 || reason.length > 500) return;
    set(k, { busy: true, text: "" });
    void api.rejectPlan(i.id, reason).then(
      () => { set(k, { busy: false, text: S.inbox.receipt.rejected }); P.reload(); },
      () => { set(k, { busy: false, text: S.inbox.receipt.notDone }); },
    );
  };

  const groups = P.projects.map((p) => ({ p, items: live.filter((i) => P.byCampaign[i.campaign] === p.id) })).filter((g) => g.items.length > 0);

  const card = (i: InboxItem, isSnoozed: boolean) => {
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
              </>
            )}
            {i.plan_sha256 && <p>{S.inbox.planHash}: <code className="v-ibx__hash">{i.plan_sha256}</code></p>}
            {i.invalid && <p role="note">{i.invalid}</p>}
          </div>
        )}
        {i.kind !== "plan" && i.body && <p className="v-ibx__body">{i.body}</p>}
        <div className="v-ibx__acts">
          <Button size="sm" variant="ghost" onClick={() => open(i)}>{S.inbox.open}</Button>
          {i.kind === "plan" && !i.invalid && i.plan_sha256 && (
            <Button size="sm" disabled={o?.busy} onClick={() => approve(i)}>{S.inbox.approve}</Button>
          )}
          {isSnoozed
            ? <Button size="sm" variant="ghost" onClick={() => setUntil((u) => { const { [k]: _, ...rest } = u; return rest; })}>{S.inbox.unsnooze}</Button>
            : <Button size="sm" variant="ghost" onClick={() => setUntil((u) => ({ ...u, [k]: Date.now() + SNOOZE_MS }))}>{S.inbox.snooze}</Button>}
        </div>
        {i.kind === "plan" && (
          <div className="v-ibx__rej">
            <input type="text" placeholder={S.inbox.reason} aria-label={S.inbox.reason} value={reason} onChange={(e) => setReasons((m) => ({ ...m, [k]: e.target.value }))} />
            <Button size="sm" variant="ghost" disabled={o?.busy || reason.trim().length < 1 || reason.trim().length > 500} onClick={() => reject(i)}>{S.inbox.reject}</Button>
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
      {P.state === "ok" && live.length === 0 && later.length === 0 && <div className="v-board__state"><Empty title={S.inbox.empty}>{S.inbox.emptyHint}</Empty></div>}
      {groups.map(({ p, items }) => (
        <section key={p.id} className="v-inbox__group" aria-label={p.name}>
          <h2 className="v-inbox__h">{p.name}</h2>
          {items.map((i) => card(i, false))}
        </section>
      ))}
      {later.length > 0 && (
        <section className="v-inbox__group">
          <button type="button" className="v-inbox__more" aria-expanded={showSnoozed} onClick={() => setShowSnoozed((x) => !x)}>{S.inbox.snoozed(later.length)}</button>
          {showSnoozed && later.map((i) => card(i, true))}
        </section>
      )}
    </section>
  );
}
