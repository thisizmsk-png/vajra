// Settings > Providers, the multi-provider part: several accounts (billing, override warning, usage, models), Add / Replace /
// Remove for an api_key account, and the advanced base URL. The key is typed into a password field, handed to the core once and
// dropped: it is never kept after Save, never re-rendered, and no stored value comes back to show (the record says `saved` only).
import { useEffect, useId, useState } from "react";
import { AlertDialog, Badge, Button, Input, Sheet, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { api } from "../ipc";
import { isNewModel, modelLine, modelName, pct, warnWindow } from "../accountsContract";
import type { Account, AccountModel, Usage, UsageWindow } from "../accountsContract";
import { BASE_URL_SAVE_WIRED, baseUrlProblem } from "../accountsContract";
import { useAccounts } from "../providerData";
import type { Accounts } from "../providerData";
import { ProviderIcon } from "./ProviderIcon";
import { problemDetail } from "./states";
import "./providers.css";
import "./accounts.css";

const C = S.accounts;

const meterText = (w: UsageWindow): string => {
  if (w.kind === "credit" && w.used_usd !== null && w.limit_usd !== null) return C.credit(w.used_usd, w.limit_usd);
  const p = pct(w);
  return p === null ? C.windowNotReported : C.used(p);
};

/** Usage windows as meters. A window the account does not report says so; nothing is estimated. */
export function UsageMeters({ account, usage }: { account: Account; usage: Usage | undefined }) {
  const windows = usage?.windows ?? [];
  if (windows.length === 0) return <Text variant="body-sm" muted>{C.notReported}</Text>;
  return (
    <ul className="v-usages" aria-label={C.meters(account.label)}>
      {windows.map((w) => {
        const p = pct(w);
        const hot = warnWindow({ account: account.id, read_at: null, windows: [w] }) !== null;
        const name = C.window[w.kind] ?? w.kind;
        return (
          <li key={w.kind} className={hot ? "v-usage v-usage--warn" : "v-usage"}>
            <div className="v-usage__head">
              <span>{name}</span>
              <span className="v-num">{meterText(w)}</span>
              {hot && <Badge status="warning">{C.nearLimit}</Badge>}
            </div>
            {p !== null && (
              <div className="v-usage__bar" role="meter" aria-label={`${account.label} ${name}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={p} aria-valuetext={meterText(w)}>
                <i style={{ width: `${p}%` }} />
              </div>
            )}
            <Text variant="body-sm" muted>{w.resets_at ? C.resets(w.resets_at) : w.kind === "credit" ? C.creditLimitAt : ""}</Text>
          </li>
        );
      })}
    </ul>
  );
}

function ModelRow({ account, model }: { account: Account; model: AccountModel }) {
  const check = model.tool_check === "failed" ? C.toolCheck.failed(model.checked_on) : C.toolCheck[model.tool_check];
  return (
    <li className="v-model">
      <ProviderIcon family={model.family} model={model.id} via={account.provider === "openrouter" ? "openrouter" : undefined} />
      <span className="v-model__line">
        <span data-model-line>{modelLine(account, model.id, C.defaultModel)}</span>
        <span className="v-num v-model__id">{model.id}</span>
      </span>
      {isNewModel(account) && <Badge status="info">{C.newBadge}</Badge>}
      <Badge status={model.tool_check === "passed" ? "success" : model.tool_check === "failed" ? "danger" : "warning"}>{check}</Badge>
    </li>
  );
}

export function BaseUrl({ account, onSaved }: { account: Account; onSaved: () => void }) {
  const [url, setUrl] = useState(account.base_url ?? "");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const save = async () => {
    const p = baseUrlProblem(url);
    if (p) return setError(C.baseUrlProblem[p]);
    setBusy(true);
    setError("");
    try {
      await api.providerBaseUrlSave(account.id, url.trim());
      onSaved();
    } catch (e) {
      setError(`${C.baseUrlFailed} ${problemDetail(e)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <details className="v-acct__adv">
      <summary>{C.advanced}</summary>
      <Input label={C.baseUrlLabel} value={url} onChange={(v) => { setUrl(v); setError(""); }} hint={C.baseUrlHint} error={error || undefined} placeholder="https://" />
      <Button size="sm" busy={busy} onClick={() => void save()}>{C.baseUrlSave}</Button>
    </details>
  );
}

export function AccountRow({ account: a, usage, onReload, onKey, onRemove }: { account: Account; usage: Usage | undefined; onReload: () => void; onKey: (a: Account) => void; onRemove: (a: Account) => void }) {
  const id = useId();
  const billing = a.billing.kind === "subscription" ? C.billing.subscription(a.billing.account_label)
    : a.billing.kind === "gateway" ? C.billing.gateway(a.billing.account_label) : a.billing.kind === "api_key" ? C.billing.api_key : C.billing.unknown;
  const apiKey = a.credential.source !== "cli_login";
  const outside = a.harness === "codex";
  return (
    <li className="v-acct" aria-labelledby={id}>
      <ProviderIcon family={a.provider} large />
      <div className="v-acct__main">
        <h3 id={id} className="v-acct__name">{a.label}</h3>
        <ul className="v-acct__facts">
          <li>{C.state[a.state] ?? C.state.cannot_tell}</li>
          <li>{billing}</li>
          {apiKey && <li>{C.runsOn(C.harness[a.harness] ?? a.harness)}</li>}
        </ul>
        {a.billing.override_warning && <p className="v-acct__warn"><Badge status="warning">{C.overrideWarning(a.billing.override_warning)}</Badge></p>}
        <UsageMeters account={a} usage={usage} />
        <Text variant="body-sm" muted>{outside ? C.outsideGuard : C.guarded}</Text>
        {a.models.length > 0 && (
          <ul className="v-models" aria-label={C.models(a.label)}>
            {a.models.map((m) => <ModelRow key={m.id} account={a} model={m} />)}
          </ul>
        )}
        {apiKey && BASE_URL_SAVE_WIRED && <BaseUrl key={a.base_url ?? ""} account={a} onSaved={onReload} />}
      </div>
      <div className="v-acct__actions">
        {!apiKey && <Button size="sm" variant="ghost" aria-label={`${C.check}: ${a.label}`} onClick={onReload}>{C.check}</Button>}
        {apiKey && <Button size="sm" variant="ghost" onClick={() => onKey(a)} aria-label={`${a.credential.saved ? C.replace : C.add}: ${a.label}`}>{a.credential.saved ? C.replace : C.add}</Button>}
        {apiKey && a.credential.saved && <Button size="sm" variant="ghost" aria-label={`${C.remove}: ${a.label}`} onClick={() => onRemove(a)}>{C.remove}</Button>}
      </div>
    </li>
  );
}

/** The add / replace sheet. The key lives in this component's state only while the sheet is open and is cleared on Save, Cancel and Esc. */
export function KeySheet({ account, open, codexFound, onClose, onSaved }: { account: Account | null; open: boolean; codexFound: boolean; onClose: () => void; onSaved: () => void }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { if (!open) { setKey(""); setError(""); } }, [open]);
  const close = () => { setKey(""); setError(""); onClose(); };
  const save = async () => {
    if (!account || key.trim() === "") return;
    setBusy(true);
    setError("");
    try {
      // Save writes a first key and refuses a second; a saved account's key is overwritten by Replace.
      await (account.credential.saved ? api.providerKeyReplace : api.providerKeySave)(account.id, key.trim());
      setKey("");
      onSaved();
    } catch (e) {
      setError(`${C.keyFailed} ${problemDetail(e)}`);
    } finally {
      setBusy(false);
    }
  };
  const title = account?.credential.saved ? C.replaceTitle : C.keyTitle;
  return (
    <Sheet open={open} onClose={close} title={title} side="right">
      <Input type="password" label={C.keyLabel} value={key} onChange={setKey} error={error || undefined} hint={C.keyHint} />
      <Text variant="body-sm" muted>{C.keyLimit}</Text>
      <Text variant="body-sm" muted>{C.keyHarness(codexFound)}</Text>
      <div className="v-dialog__actions">
        <Button variant="ghost" onClick={close}>{C.cancel}</Button>
        <Button variant="primary" busy={busy} disabled={key.trim() === ""} onClick={() => void save()}>{busy ? C.saving : C.save}</Button>
      </div>
    </Sheet>
  );
}

/** Settings > Providers > Accounts. */
export function AccountsPanel({ accounts }: { accounts: Accounts }) {
  const { list, usage, error, reload } = accounts;
  const [keyFor, setKeyFor] = useState<Account | null>(null);
  const [open, setOpen] = useState(false);
  const [removing, setRemoving] = useState<Account | null>(null);
  const [removeError, setRemoveError] = useState("");
  const codexFound = !!list?.some((a) => a.harness === "codex" && a.credential.source === "cli_login" && a.state !== "not_installed");
  const gateway = list?.find((a) => a.credential.source === "api_key") ?? null;
  const openKey = (a: Account | null) => { setKeyFor(a); setOpen(true); };
  const remove = async () => {
    if (!removing) return;
    try {
      await api.providerKeyRemove(removing.id);
      setRemoving(null);
      setRemoveError("");
      reload();
    } catch (e) {
      setRemoveError(problemDetail(e));
    }
  };
  return (
    <section aria-labelledby="v-settings-accounts" className="v-panel v-settings__checks">
      <div className="v-panel__bar">
        <h2 id="v-settings-accounts" className="v-panel__title">{C.title}</h2>
        {list && <Text as="span" variant="body-sm" muted>{C.count(list.length)}</Text>}
        {gateway && !gateway.credential.saved && <span className="v-panel__end"><Button size="sm" onClick={() => openKey(gateway)}>{C.add}</Button></span>}
      </div>
      <div className="v-panel__body">
        {error && <p role="alert">{error}</p>}
        {!list && !error && <p role="status">{C.checking}</p>}
        {list && <ul className="v-acct__list">{list.map((a) => (
          <AccountRow key={a.id} account={a} usage={usage.find((u) => u.account === a.id)} onReload={reload} onKey={openKey} onRemove={(x) => { setRemoveError(""); setRemoving(x); }} />
        ))}</ul>}
        <Text variant="body-sm" muted>{C.tileLegend}</Text>
      </div>
      <KeySheet account={keyFor} open={open} codexFound={codexFound} onClose={() => setOpen(false)} onSaved={() => { setOpen(false); reload(); }} />
      <AlertDialog open={!!removing} title={C.removeTitle} confirmLabel={C.removeConfirm} destructive onConfirm={() => void remove()} onCancel={() => setRemoving(null)}>
        <Text variant="body">{C.removeBody}</Text>
        {removeError && <p role="alert">{removeError}</p>}
      </AlertDialog>
    </section>
  );
}
