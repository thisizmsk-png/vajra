// The pickers shared by the project's agents (Settings) and the proposal card's override menu: every option is an allowed
// choice and reads "Model via provider"; effort is offered only where the account has an effort setting.
import { allowedChoices, choiceKey, modelLine } from "../accountsContract";
import type { Account, AgentPick, Agents } from "../accountsContract";
import { Text } from "../../components/ui/base";
import { S } from "../../strings";
import "./accounts.css";

const C = S.agents;
const WORD = S.accounts.defaultModel;

/** Every entry a project could allow: a CLI account's own default model, and each model enabled on an account. */
export function allEntries(accounts: Account[]): Array<{ account: Account; model: string | null }> {
  return accounts.flatMap((a) => [
    ...(a.credential.source === "cli_login" ? [{ account: a, model: null as string | null }] : []),
    ...a.models.map((m) => ({ account: a, model: m.id as string | null })),
  ]);
}

export const sameEntry = (a: { account: string; model: string | null }, b: { account: string; model: string | null }): boolean => a.account === b.account && a.model === b.model;

export function PickRow({ label, pick, agents, accounts, onChange, inherit, hideLabel }: { label: string; pick: AgentPick | null; agents: Pick<Agents, "allowed">; accounts: Account[]; onChange: (p: AgentPick | null) => void; inherit?: boolean; hideLabel?: boolean }) {
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
      <label className="v-agent__label"><span className={hideLabel ? "v-agent__sr" : undefined}>{label}</span>
        <select aria-label={C.pickLabel(label)} value={value} onChange={(e) => set(e.target.value)}>
          {inherit && <option value="">{C.useDefault}</option>}
          {stale && pick && <option value={value} disabled>{account ? modelLine(account, pick.model, WORD) : S.accounts.modelVia(pick.model ?? WORD, pick.account)}</option>}
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

