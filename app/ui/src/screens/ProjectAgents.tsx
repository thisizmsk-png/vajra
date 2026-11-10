// Project scope in Settings > Providers: the default agent, a default per crew role (provider and model, effort) and the allowed
// set. The Director-General starts from the role default and may only pick from `allowed`, so every picker here offers the allowed
// choices and nothing else, and an allowed entry that a default still uses cannot be turned off. Every option reads "Model · provider".
import { useEffect, useId, useState } from "react";
import { Button, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { api } from "../ipc";
import { allowedChoices, choiceKey, coveredByAccount, modelLine, parseAgents, ROLES } from "../accountsContract";
import type { Account, AgentPick, Agents } from "../accountsContract";
import { ProviderIcon } from "./ProviderIcon";
import { problemDetail } from "./states";
import "./providers.css";

const C = S.agents;
const WORD = S.accounts.defaultModel;

/** Every entry a project could allow: a CLI account's own default model, and each model enabled on an account. */
export function allEntries(accounts: Account[]): Array<{ account: Account; model: string | null }> {
  return accounts.flatMap((a) => [
    ...(a.credential.source === "cli_login" ? [{ account: a, model: null as string | null }] : []),
    ...a.models.map((m) => ({ account: a, model: m.id as string | null })),
  ]);
}

const sameEntry = (a: { account: string; model: string | null }, b: { account: string; model: string | null }): boolean => a.account === b.account && a.model === b.model;

function PickRow({ label, pick, agents, accounts, onChange, inherit }: { label: string; pick: AgentPick | null; agents: Agents; accounts: Account[]; onChange: (p: AgentPick | null) => void; inherit?: boolean }) {
  const choices = allowedChoices(accounts, agents.allowed);
  const account = pick ? accounts.find((a) => a.id === pick.account) : undefined;
  const value = pick ? choiceKey(pick.account, pick.model) : "";
  // A saved pick the allowed list no longer offers stays visible but cannot be chosen again; it is never offered to anyone else.
  const stale = pick && !choices.some((c) => sameEntry({ account: c.account.id, model: c.model }, pick));
  const set = (key: string) => {
    if (key === "") return onChange(null);
    const c = choices.find((x) => choiceKey(x.account.id, x.model) === key);
    if (!c) return;
    const effort = pick?.effort && c.account.efforts.includes(pick.effort) ? pick.effort : null;
    onChange({ account: c.account.id, model: c.model, effort });
  };
  return (
    <div className="v-agent">
      <label className="v-agent__label"><span>{label}</span>
        <select aria-label={C.pickLabel(label)} value={value} onChange={(e) => set(e.target.value)}>
          {inherit && <option value="">{C.useDefault}</option>}
          {stale && pick && <option value={value} disabled>{account ? modelLine(account, pick.model, WORD) : `${pick.model ?? WORD} · ${pick.account}`}</option>}
          {choices.map((c) => <option key={choiceKey(c.account.id, c.model)} value={choiceKey(c.account.id, c.model)}>{modelLine(c.account, c.model, WORD)}</option>)}
        </select>
      </label>
      {pick && account && (account.efforts.length > 0
        ? <label className="v-agent__effort"><span className="v-agent__sr">{C.effortLabel(label)}</span>
            <select aria-label={C.effortLabel(label)} value={pick.effort ?? ""} onChange={(e) => onChange({ ...pick, effort: e.target.value || null })}>
              <option value="">{C.effortDefault}</option>
              {account.efforts.map((e) => <option key={e} value={e}>{e}</option>)}
            </select></label>
        : <Text as="span" variant="body-sm" muted>{C.noEffort}</Text>)}
    </div>
  );
}

export function ProjectAgents({ projectId, projectName, accounts }: { projectId: string; projectName: string; accounts: Account[] }) {
  const heading = useId();
  const [saved, setSaved] = useState<Agents | null>(null);
  const [draft, setDraft] = useState<Agents | null>(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let live = true;
    setSaved(null); setDraft(null); setError(""); setMessage("");
    api.providerAgents(projectId).then((v) => {
      if (!live) return;
      const r = parseAgents(v);
      if (r.ok) { setSaved(r.value); setDraft(r.value); } else setError(C.readFailed);
    }, (e) => { if (live) setError(`${C.readFailed} ${problemDetail(e)}`); });
    return () => { live = false; };
  }, [projectId]);
  if (!draft || !saved) return error ? <p role="alert">{error}</p> : null;
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const edit = (next: Agents) => { setDraft(next); setMessage(""); };
  const inUse = (e: { account: string; model: string | null }) => sameEntry(draft.default, e) || Object.values(draft.roles).some((r) => sameEntry(r, e));
  const toggle = (e: { account: string; model: string | null }, accountModels: string[]) => {
    // Narrowing under an account-wide allowance (a model row, or a CLI login's default row, which is that entry): it becomes explicit
    // entries for every model except the one turned off, so unchecking one row never flips the others.
    if (coveredByAccount(draft.allowed, e.account)) {
      const rest = accountModels.filter((id) => id !== e.model).map((id) => ({ account: e.account, model: id }));
      return edit({ ...draft, allowed: [...draft.allowed.filter((x) => x.account !== e.account || x.model !== null), ...rest.filter((r) => !draft.allowed.some((x) => sameEntry(x, r)))] });
    }
    const on = draft.allowed.some((x) => sameEntry(x, e));
    edit({ ...draft, allowed: on ? draft.allowed.filter((x) => !sameEntry(x, e)) : [...draft.allowed, e] });
  };
  const setRole = (role: string, p: AgentPick | null) => {
    const roles = { ...draft.roles };
    if (p) roles[role] = p; else delete roles[role];
    edit({ ...draft, roles });
  };
  const save = async () => {
    setBusy(true); setError(""); setMessage("");
    try {
      const r = parseAgents(await api.providerAgentsSave(projectId, { default: draft.default, roles: draft.roles, allowed: draft.allowed }));
      if (!r.ok) throw new Error("unreadable");
      setSaved(r.value); setDraft(r.value); setMessage(C.saved);
    } catch (e) {
      setError(`${C.saveFailed} ${e instanceof Error ? "" : problemDetail(e)}`.trim());
    } finally {
      setBusy(false);
    }
  };
  return (
    <section aria-labelledby={heading} className="v-panel v-settings__checks">
      <div className="v-panel__bar">
        <h2 id={heading} className="v-panel__title">{projectName}</h2>
        <Text as="span" variant="body-sm" muted>{C.scope}</Text>
      </div>
      <div className="v-panel__body v-agents">
        <Text variant="body-sm" muted>{C.lede}</Text>
        <PickRow label={C.defaultAgent} pick={draft.default} agents={draft} accounts={accounts} onChange={(p) => p && edit({ ...draft, default: p })} />
        <h3 className="v-agents__h">{C.roleDefaults}</h3>
        <Text variant="body-sm" muted>{C.roleHint}</Text>
        {ROLES.map((r) => <PickRow key={r} label={C.role[r]} pick={draft.roles[r] ?? null} inherit agents={draft} accounts={accounts} onChange={(p) => setRole(r, p)} />)}
        <h3 className="v-agents__h">{C.allowed}</h3>
        <ul className="v-allowed">
          {allEntries(accounts).map((e) => {
            const key = choiceKey(e.account.id, e.model);
            const covered = e.model !== null && coveredByAccount(draft.allowed, e.account.id);
            const on = covered || draft.allowed.some((x) => sameEntry(x, { account: e.account.id, model: e.model }));
            // Narrowing a CLI login's account-wide allowance would drop its own default, so that stays locked while a default uses it.
            const locked = on && (inUse({ account: e.account.id, model: e.model }) || (covered && e.account.credential.source === "cli_login" && inUse({ account: e.account.id, model: null })));
            return (
              <li key={key}>
                <label>
                  <input type="checkbox" checked={on} disabled={locked} aria-describedby={locked ? `${heading}-${key}` : undefined} onChange={() => toggle({ account: e.account.id, model: e.model }, e.account.models.map((m) => m.id))} />
                  <ProviderIcon family={e.account.models.find((m) => m.id === e.model)?.family ?? e.account.provider} via={e.model && e.account.provider === "openrouter" ? "openrouter" : undefined} />
                  <span data-model-line>{modelLine(e.account, e.model, WORD)}</span>
                </label>
                {locked && <span id={`${heading}-${key}`} className="v-text v-text--body-sm v-text--muted">{C.inUse}</span>}
              </li>
            );
          })}
        </ul>
        {error && <p role="alert">{error}</p>}
        {message && <p role="status">{message}</p>}
        <div className="v-pref__actions">
          <Button variant="primary" disabled={!dirty} busy={busy} onClick={() => void save()}>{C.save}</Button>
          <Button disabled={!dirty || busy} onClick={() => { setDraft(saved); setMessage(""); }}>{C.discard}</Button>
        </div>
      </div>
    </section>
  );
}
