// The per-worker provider choice on a run proposal (a lazy chunk: only a proposal that carries `workers` loads it). Each worker
// has a chip (icon, "Model · provider", effort), a billing line, the Director-General's reason, the limits that provider has
// and a usage warning at 80%. The pickers offer only what the project allows; a changed pick says "Changed by you", and
// nothing starts until Start, which carries the changes. The proposal hash covers `workers`: a mismatch is refused by the core.
import { useEffect, useMemo, useState } from "react";
import { S } from "../../../strings";
import { api } from "../../ipc";
import type { InboxItem } from "../../ipc";
import { modelLine, parseAgents, parseProposalWorkers, pct, warnWindow } from "../../accountsContract";
import type { Account, Agents, AgentPick, ProposalOverride, Usage } from "../../accountsContract";
import { useAccounts } from "../../providerData";
import { PickRow, sameEntry } from "../AgentPickers";
import { ProviderIcon } from "../ProviderIcon";
import "../providers.css";
import "./proposalWorkers.css";

const P = S.proposalWorkers;
const same = (a: AgentPick, b: AgentPick) => sameEntry(a, b) && a.effort === b.effort;

function billingLine(a: Account | undefined): string {
  if (!a) return P.billingUnknown;
  const base = a.billing.kind === "unknown" ? P.billingUnknown : a.billing.account_label ?? S.workerProvider.billing[a.billing.kind];
  return a.billing.override_warning ? `${base} · ${S.accounts.overrideShort(a.billing.override_warning)}` : base;
}

function warnLine(a: Account | undefined, u: Usage | undefined): string | null {
  const w = warnWindow(u);
  const p = w ? pct(w) : null;
  return a && w && p !== null ? S.accounts.warn80(a.label, S.accounts.window[w.kind] ?? w.kind, p, w.resets_at) : null;
}

export default function ProposalWorkers({ item, onOverrides }: { item: InboxItem; onOverrides: (o: ProposalOverride[]) => void }) {
  const workers = useMemo(() => parseProposalWorkers((item as { workers?: unknown }).workers), [item]);
  const { list, usage } = useAccounts(true);
  const [allowed, setAllowed] = useState<Pick<Agents, "allowed"> | null | "failed">(null);
  const [picks, setPicks] = useState<Record<string, AgentPick>>({});
  useEffect(() => {
    let live = true;
    if (!item.project_id) { setAllowed("failed"); return; }
    api.providerAgents(item.project_id).then((v) => { if (live) { const r = parseAgents(v); setAllowed(r.ok ? r.value : "failed"); } }, () => { if (live) setAllowed("failed"); });
    return () => { live = false; };
  }, [item.project_id]);
  const current = (slot: string): AgentPick => picks[slot] ?? workers.find((w) => w.slot === slot)!.pick;
  const changed = workers.filter((w) => picks[w.slot] && !same(picks[w.slot], w.pick));
  useEffect(() => {
    onOverrides(changed.map((w) => ({ slot: w.slot, account: picks[w.slot].account, model: picks[w.slot].model, effort: picks[w.slot].effort })));
    // `changed` is derived from `picks`; the callback is the parent's setter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [picks]);
  if (workers.length === 0) return null;
  const acct = (id: string) => list?.find((a) => a.id === id);
  const ok = list && allowed && allowed !== "failed" ? { list, allowed } : null;
  // The footer: who pays for what, which workers leave the Vajra guard, and any account that is nearly full.
  const byAcct = new Map<string, number>();
  for (const w of workers) byAcct.set(current(w.slot).account, (byAcct.get(current(w.slot).account) ?? 0) + 1);
  const parts = [...byAcct].map(([id, n]) => {
    const a = acct(id);
    if (!a) return P.login(id, n);
    return a.billing.kind === "subscription" ? P.plan(a.label, n) : a.credential.source === "cli_login" ? P.login(a.label, n) : P.key(a.label, n);
  });
  const outside = workers.filter((w) => acct(current(w.slot).account)?.harness === "codex").length;
  const warnings = [...byAcct.keys()].map((id) => warnLine(acct(id), usage.find((u) => u.account === id))).filter((x): x is string => !!x);
  return (
    <section className="v-pw" aria-label={P.title}>
      {allowed === "failed" && <p className="v-pw__note">{P.readFailed}</p>}
      <table className="v-pw__table">
        <thead><tr><th scope="col">{P.worker}</th><th scope="col">{P.runsOn}</th><th scope="col">{P.why}</th></tr></thead>
        <tbody>
          {workers.map((w) => {
            const pick = current(w.slot);
            const a = acct(pick.account);
            const isChanged = !!picks[w.slot] && !same(picks[w.slot], w.pick);
            const proposed = (() => { const pa = acct(w.pick.account); return pa ? modelLine(pa, w.pick.model, S.accounts.defaultModel) : w.pick.account; })();
            const warn = warnLine(a, usage.find((u) => u.account === pick.account));
            const family = a?.models.find((m) => m.id === pick.model)?.family ?? a?.provider ?? pick.account;
            return (
              <tr key={w.slot}>
                <th scope="row"><span className="v-pw__role">{S.agents.role[w.role] ?? w.role}</span><span className="v-pw__task">{w.task}</span></th>
                <td>
                  {ok ? (
                    <div className="v-pw__pick">
                      <ProviderIcon family={family} model={pick.model} via={a && a.provider === "openrouter" ? "openrouter" : undefined} />
                      <PickRow label={`${S.agents.role[w.role] ?? w.role}: ${w.task}`} hideLabel pick={pick} agents={ok.allowed} accounts={ok.list}
                        onChange={(p) => p && setPicks((x) => ({ ...x, [w.slot]: p }))} />
                    </div>
                  ) : (
                    <div className="v-pw__pick"><ProviderIcon family={family} model={pick.model} /><span data-model-line>{a ? modelLine(a, pick.model, S.accounts.defaultModel) : pick.account}</span>{pick.effort && <span className="v-num">{pick.effort}</span>}</div>
                  )}
                  <p className="v-pw__bill">{billingLine(a)}</p>
                  {a?.harness === "codex" && <p className="v-pw__lim">{P.limits}</p>}
                  {warn && <p className="v-pw__warn">{warn}</p>}
                  {isChanged && <p className="v-pw__chg">{P.changed(proposed)}</p>}
                </td>
                <td className="v-pw__why">{w.why}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {ok && <p className="v-pw__note">{P.notAllowed}</p>}
      <p className="v-pw__sum">
        {parts.length > 0 && P.billed(parts)} {outside > 0 && P.outside(outside)} {warnings.join(" ")}
      </p>
    </section>
  );
}

