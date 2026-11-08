import { useEffect, useState } from "react";
import { Button } from "../../../components/ui/base";
import { usePreferenceScope } from "../../preferences";
import { SHORTCUT_CHOICES, shortcutKeys } from "../../preferenceContract";
import type { PreferenceKey, PreferencePatch, Preferences } from "../../preferenceContract";
import "./preferences.css";

export function PreferenceForm({ projectId, projectName, section, onDirty }: { projectId: string | null; projectName?: string; section: string; onDirty?: (dirty: boolean) => void }) {
  const prefs = usePreferenceScope(projectId, false);
  const [patch, setPatch] = useState<PreferencePatch>({}); const [message, setMessage] = useState("");
  const dirty = Object.keys(patch).length > 0;
  useEffect(() => { setPatch({}); setMessage(""); }, [projectId, prefs.view?.rev]);
  useEffect(() => { onDirty?.(dirty); }, [dirty, onDirty]);
  if (!prefs.view) return <div className="v-pref">{prefs.error ? <><p role="alert">{prefs.error}</p><Button onClick={prefs.reload}>Reload preferences</Button></> : <p role="status">Reading preferences…</p>}</div>;
  const view = prefs.view;
  const value = <K extends PreferenceKey>(key: K): Preferences[K] => patch.inherit?.includes(key) ? view.global[key] : (patch[key] ?? view.effective[key]) as Preferences[K];
  const change = <K extends PreferenceKey>(key: K, v: Preferences[K]) => { setMessage(""); setPatch(p => ({ ...p, [key]: v, ...(p.inherit ? { inherit: p.inherit.filter(k => k !== key) } : {}) })); };
  const inherit = (key: PreferenceKey) => { setMessage(""); setPatch(p => { const next = { ...p, inherit: [...new Set([...(p.inherit ?? []), key])] }; delete next[key]; return next; }); };
  const source = (key: PreferenceKey) => key in patch || patch.inherit?.includes(key) ? "Unsaved change" : view.sources[key] === "project" ? "Project override" : "Global default";
  const select = <K extends PreferenceKey>(key: K, label: string, choices: readonly (readonly [string, string, boolean?])[]) => (
    <div className="v-pref__row" key={key}>
      <label><span>{label}<small>{source(key)}</small></span><select aria-label={label} value={String(value(key))} disabled={prefs.busy} onChange={e => change(key, (typeof view.effective[key] === "number" ? Number(e.target.value) : e.target.value) as Preferences[K])}>
        {choices.map(([v, text, disabled]) => <option key={v} value={v} disabled={disabled}>{text}</option>)}
      </select></label>
      {projectId && <Button size="sm" variant="ghost" disabled={prefs.busy} onClick={() => inherit(key)}>Use global {label}</Button>}
    </div>
  );
  const save = async (reset = false) => { setMessage(""); if (await prefs.save(reset ? {} : patch, reset)) { setPatch({}); setMessage("Saved."); } };
  return <section className="v-pref" aria-label="Saved preferences">
    <p>{projectId ? `Overrides for ${projectName ?? projectId}` : "Global defaults"}. Changes apply only after Save.</p>
    {section === "appearance" && select("theme", "Theme", [["system", "Follow system"], ["dark", "Dark"], ["light", "Light"]])}
    {section === "general" && <>
      {select("editor", "Default editor", [["system", "System default"], ["code", "Editor CLI"]])}
      {select("shell", "Terminal shell", [["login", "Login shell"], ["sh", "/bin/sh"], ["bash", "/bin/bash"], ["zsh", "/bin/zsh"]])}
      <p>Launch preferences are saved for supported new opens. Running sessions keep their launch settings.</p>
    </>}
    {section === "models" && <>
      {select("agent", "Default agent", [["claude", "Claude Code"], ["codex", "Codex CLI — availability check required", true]])}
      <p>Model: {view.effective.model || "Native CLI default"}. Additional agent and model selection requires a supported adapter.</p>
      {select("workers", "Concurrent workers", Array.from({ length: 16 }, (_, i) => [String(i + 1), String(i + 1)] as const))}
      {select("rounds", "Fix rounds", [["1", "1"], ["2", "2"], ["3", "3"]])}
      <p>Defaults apply to new tasks. System limits and native permissions still apply.</p>
    </>}
    {section === "notifications" && <div className="v-pref__row"><label><span>Task notifications<small>{source("notifications")}</small></span><input type="checkbox" checked={value("notifications")} disabled={prefs.busy} onChange={e => change("notifications", e.target.checked)} /></label>
      {projectId && <Button size="sm" variant="ghost" disabled={prefs.busy} onClick={() => inherit("notifications")}>Use global Task notifications</Button>}
      <p>This preference does not grant OS notification permission or include code and prompts in notifications.</p>
    </div>}
    {section === "keyboard" && (["palette", "settings"] as const).map(key => <div className="v-pref__row" key={key}><label><span>{key === "palette" ? "Search shortcut" : "Settings shortcut"}<small>{source("shortcuts")}</small></span>
      <select aria-label={key === "palette" ? "Search shortcut" : "Settings shortcut"} value={value("shortcuts")[key]} disabled={prefs.busy} onChange={e => change("shortcuts", { ...value("shortcuts"), [key]: e.target.value })}>
        {SHORTCUT_CHOICES.map(k => <option key={k} value={k}>{shortcutKeys(k).join(" ")}</option>)}
      </select></label>{projectId && <Button size="sm" variant="ghost" disabled={prefs.busy} onClick={() => inherit("shortcuts")}>Use global {key === "palette" ? "Search shortcut" : "Settings shortcut"}</Button>}</div>)}
    {prefs.error && <p role="alert">{prefs.error}</p>}
    {message && <p role="status">{message}</p>}
    <div className="v-pref__actions">
      <Button variant="primary" disabled={!dirty || prefs.busy} busy={prefs.busy} onClick={() => void save()}>Save preferences</Button>
      <Button disabled={!dirty || prefs.busy} onClick={() => { setPatch({}); setMessage(""); }}>Discard changes</Button>
      <Button disabled={prefs.busy} onClick={() => void save(true)}>Reset this scope</Button>
      <Button variant="ghost" disabled={prefs.busy} onClick={() => { setPatch({}); setMessage(""); prefs.reload(); }}>Reload preferences</Button>
    </div>
  </section>;
}
