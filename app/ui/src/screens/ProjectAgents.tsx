// Project scope in Settings > Providers: the default agent, a default per crew role (provider and model, effort) and the allowed
// set. The Director-General starts from the role default and may only pick from `allowed`, so every picker here offers the allowed
// choices and nothing else, and an allowed entry that a default still uses cannot be turned off. Every option reads "Model · provider".
import { useEffect, useId, useState } from "react";
import { Button, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { api } from "../ipc";
import { parseAgents, ROLES } from "../accountsContract";
import type { Account, AgentPick, Agents } from "../accountsContract";
import { PickRow, allEntries, sameEntry } from "./AgentPickers";
import { choiceKey, modelLine } from "../accountsContract";
import { ProviderIcon } from "./ProviderIcon";
import { problemDetail } from "./states";
import "./providers.css";
import "./accounts.css";

const C = S.agents;
const WORD = S.accounts.defaultModel;

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
  const toggle = (e: { account: string; model: string | null }) => {
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
            const on = draft.allowed.some((x) => sameEntry(x, { account: e.account.id, model: e.model }));
            const locked = on && inUse({ account: e.account.id, model: e.model });
            return (
              <li key={key}>
                <label>
                  <input type="checkbox" checked={on} disabled={locked} aria-describedby={locked ? `${heading}-${key}` : undefined} onChange={() => toggle({ account: e.account.id, model: e.model })} />
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
