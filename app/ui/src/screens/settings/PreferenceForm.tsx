import { useEffect, useState } from "react";
import { Button, Switch } from "../../../components/ui/base";
import { usePreferenceScope } from "../../preferences";
import { SHORTCUT_CHOICES, shortcutKeys } from "../../preferenceContract";
import type { PreferenceKey, PreferencePatch, Preferences } from "../../preferenceContract";
import { S } from "../../../strings";
import "./preferences.css";
const C = S.preferences;

/** `agents`: the provider choices for the `providers` section, as [id, words, disabled]; built from `provider_list` by Settings. */
export type AgentChoice = readonly [string, string, boolean?];
export function PreferenceForm({ projectId, projectName, section, onDirty, agents = [] }: { projectId: string | null; projectName?: string; section: string; onDirty?: (dirty: boolean) => void; agents?: readonly AgentChoice[] }) {
  const prefs = usePreferenceScope(projectId, false);
  const [patch, setPatch] = useState<PreferencePatch>({}); const [message, setMessage] = useState("");
  const dirty = Object.keys(patch).length > 0;
  // Reset during render, not in an effect: an effect runs after paint, so an edit made in that gap (the form has just
  // appeared) would be wiped. The first load of a scope clears nothing; a new scope clears the draft and the message;
  // a new revision (another window saved, or our own save landed) clears only the draft, so our own "Saved." stays.
  const scope = projectId ?? ""; const rev = prefs.view?.rev;
  const [seen, setSeen] = useState<{ scope: string; rev: number | undefined }>({ scope, rev });
  if (seen.scope !== scope || seen.rev !== rev) {
    setSeen({ scope, rev });
    if (seen.scope !== scope) { setPatch({}); setMessage(""); }
    else if (seen.rev !== undefined && rev !== undefined) setPatch({});
  }
  useEffect(() => { onDirty?.(dirty); }, [dirty, onDirty]);
  if (!prefs.view) return <div className="v-pref">{prefs.error ? <><p role="alert">{prefs.error}</p><Button onClick={prefs.reload}>{C.reload}</Button></> : <p role="status">{C.reading}</p>}</div>;
  const view = prefs.view;
  const value = <K extends PreferenceKey>(key: K): Preferences[K] => patch.inherit?.includes(key) ? view.global[key] : (patch[key] ?? view.effective[key]) as Preferences[K];
  const change = <K extends PreferenceKey>(key: K, v: Preferences[K]) => { setMessage(""); setPatch(p => ({ ...p, [key]: v, ...(p.inherit ? { inherit: p.inherit.filter(k => k !== key) } : {}) })); };
  const inherit = (key: PreferenceKey) => { setMessage(""); setPatch(p => { const next = { ...p, inherit: [...new Set([...(p.inherit ?? []), key])] }; delete next[key]; return next; }); };
  // While a save is in flight, uncertain or awaiting review, nothing may change the draft: a retry replays the original
  // intent, and an edit made after it would be cleared when the retry lands (PR 100 review).
  const locked = prefs.busy || prefs.uncertain || prefs.needsReview;
  const source = (key: PreferenceKey) => key in patch || patch.inherit?.includes(key) ? C.unsaved : view.sources[key] === "project" ? C.projectSource : C.globalSource;
  const select = <K extends PreferenceKey>(key: K, label: string, choices: readonly (readonly [string, string, boolean?])[]) => (
    <div className="v-pref__row" key={key}>
      <label><span>{label}<small>{source(key)}</small></span><select aria-label={label} value={String(value(key))} disabled={locked} onChange={e => change(key, (typeof view.effective[key] === "number" ? Number(e.target.value) : e.target.value) as Preferences[K])}>
        {choices.map(([v, text, disabled]) => <option key={v} value={v} disabled={disabled}>{text}</option>)}
      </select></label>
      {projectId && <Button size="sm" variant="ghost" disabled={locked} onClick={() => inherit(key)}>{C.inherit(label)}</Button>}
    </div>
  );
  const save = async (reset = false) => { setMessage(""); if (await prefs.save(reset ? {} : patch, reset)) { setPatch({}); setMessage(C.saved); } };
  return <section className="v-pref" aria-label={C.region}>
    <p>{C.scopeDescription(projectId ? projectName ?? projectId : undefined)}</p>
    {section === "appearance" && select("theme", C.theme, [["system", C.systemTheme], ["dark", C.dark], ["light", C.light]])}
    {/* Editor, shell and Notifications have no form yet: nothing opens an editor or a terminal, or delivers a
        notification, from those values, so a saved choice would change nothing (PR 100 review). The stored keys and their
        defaults stay in the contract until a consumer reads them. General holds only what the core reads: closing finished workers. */}
    {section === "general" && <>
      <div className="v-pref__row">
        <label><span>{C.autoClose}<small>{source("auto_close_finished")}</small></span>
          <Switch label={C.autoClose} checked={value("auto_close_finished")} onChange={v => { if (!locked) change("auto_close_finished", v); }} /></label>
        {projectId && <Button size="sm" variant="ghost" disabled={locked} onClick={() => inherit("auto_close_finished")}>{C.inherit(C.autoClose)}</Button>}
      </div>
      <p>{C.autoCloseHint}</p>
    </>}
    {section === "providers" && <>
      {/* A saved value this list does not offer (an older choice, or a check that has not answered) stays visible, never silently swapped. */}
      {select("agent", S.providers.agent, agents.some(([v]) => v === value("agent")) ? agents : [...agents, [value("agent"), value("agent"), true]])}
      <p>{S.providers.agentHint}</p>
    </>}
    {section === "models" && <>
      <p>{C.modelHint(view.effective.model || C.nativeModel)}</p>
      {select("workers", C.workers, Array.from({ length: 16 }, (_, i) => [String(i + 1), String(i + 1)] as const))}
      {select("rounds", C.rounds, [["1", "1"], ["2", "2"], ["3", "3"]])}
      <p>{C.caps(view.caps?.active_runs ?? 1, view.caps?.workers_per_run ?? 16)}</p>
      <p>{C.defaultsHint}</p>
    </>}
    {section === "keyboard" && <>
      {(["palette", "settings"] as const).map(key => <div className="v-pref__row" key={key}><label><span>{key === "palette" ? C.searchShortcut : C.settingsShortcut}<small>{source("shortcuts")}</small></span>
        <select aria-label={key === "palette" ? C.searchShortcut : C.settingsShortcut} value={value("shortcuts")[key]} disabled={locked} onChange={e => change("shortcuts", { ...value("shortcuts"), [key]: e.target.value })}>
          {SHORTCUT_CHOICES.map(k => <option key={k} value={k}>{shortcutKeys(k).join(" ")}</option>)}
        </select></label></div>)}
      {/* The shortcuts are one override in the store, so inheriting is one group-wide control that says it resets both (PR 100 review). */}
      {projectId && <Button size="sm" variant="ghost" disabled={locked} onClick={() => inherit("shortcuts")}>{C.inheritShortcuts}</Button>}
    </>}
    {prefs.error && <p role="alert">{prefs.error}</p>}
    {prefs.receipt && <p role="status">{C.receipt(prefs.receipt.state, prefs.receipt.action_id, prefs.receipt.reason)}</p>}
    {prefs.uncertain && <Button disabled={prefs.busy} onClick={() => { void prefs.retry().then(ok => { if (ok) { setPatch({}); setMessage(C.saved); } }); }}>{C.retry}</Button>}
    {message && <p role="status">{message}</p>}
    <div className="v-pref__actions">
      <Button variant="primary" disabled={!dirty || prefs.busy || prefs.uncertain || prefs.needsReview} busy={prefs.busy} onClick={() => void save()}>{C.save}</Button>
      {/* Not while a save's outcome is uncertain: Retry would still apply the "discarded" draft (PR 100 review). */}
      <Button disabled={!dirty || locked} onClick={() => { setPatch({}); setMessage(""); }}>{C.discard}</Button>
      <Button disabled={prefs.busy || prefs.uncertain || prefs.needsReview} onClick={() => void save(true)}>{C.reset}</Button>
      <Button variant="ghost" disabled={prefs.busy} onClick={() => { setPatch({}); setMessage(""); prefs.reload(); }}>{C.reload}</Button>
    </div>
  </section>;
}
