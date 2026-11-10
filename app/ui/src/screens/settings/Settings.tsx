// Settings in the project shell: explicit global defaults and per-project overrides.
// Provider availability remains a separate check; preferences never grant execution permissions.
import { useCallback, useEffect, useState } from "react";
import { Kbd, Text } from "../../../components/ui/base";
import { ActionBar } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api } from "../../ipc";
import type { Preflight } from "../../ipc";
import { useCommands } from "../../palette";
import { toHash, useRoute } from "../../router";
import { CheckedAt, LoadingRows, Problem, problemDetail } from "../states";
import { PreflightList } from "../PreflightList";
import { PreferenceForm } from "./PreferenceForm";
import type { AgentChoice } from "./PreferenceForm";
import { ProviderCard, useProviders } from "../ProviderControls";
import type { ProviderInfo } from "../../providerContract";
import { useCurrentProject } from "../../projects/current";
import { DEFAULT_PREFERENCES, shortcutKeys } from "../../preferenceContract";
import { preferenceScope, usePreferences } from "../../preferences";

export const SECTIONS = ["providers", "general", "models", "mobile", "notifications", "appearance", "keyboard", "diagnostics"] as const;
type Section = (typeof SECTIONS)[number];
const isSection = (s: string | undefined): s is Section => !!s && (SECTIONS as readonly string[]).includes(s);

/** The shortcuts the shell really handles (Shell.tsx). One list, so Settings and the keys cannot drift apart silently. */
export const SHORTCUTS: ReadonlyArray<{ label: string; keys: string[] }> = [
  { label: S.settings.keys.palette, keys: ["⌘", "K"] },
  { label: S.settings.keys.project, keys: ["⌘", "1"] },
  { label: S.settings.keys.column, keys: ["⌘", "B"] },
  { label: S.settings.keys.inspector, keys: ["⌘", "I"] },
  { label: S.settings.keys.settings, keys: ["⌘", ","] },
  { label: S.settings.keys.start, keys: ["⌘", "↵"] },
];

function useChecks(on: boolean, also?: () => void) {
  const [pre, setPre] = useState<Preflight | null>(null);
  const [err, setErr] = useState("");
  const [at, setAt] = useState<Date | null>(null);
  const run = useCallback(() => {
    setAt(null);
    api.preflight().then((p) => { setPre(p); setErr(""); }, (e) => setErr(problemDetail(e))).finally(() => setAt(new Date()));
  }, []);
  useEffect(() => { if (on) run(); }, [on, run]);
  useCommands(on ? [{ id: "recheck", label: S.settings.recheck, run: () => { run(); also?.(); } }] : []);
  return { pre, err, at, run };
}

export function Settings() {
  const [route] = useRoute();
  const prefs = usePreferences();
  const { project } = useCurrentProject();
  const section: Section = isSection(route.params.section) ? route.params.section : "providers";
  const meta = S.settings.sections[section];
  const providers = useProviders(section === "providers");
  const checks = useChecks(section === "providers", providers.reload);
  const recheck = () => { checks.run(); providers.reload(); };
  return (
    <div className="v-screen v-settings">
      <div className="v-settings__grid">
        <nav className="v-settings__nav" aria-label={S.settings.sectionsLabel}>
          {SECTIONS.map((s) => (
            <a key={s} href={toHash("settings", s === "providers" ? {} : { section: s }, route.project)} className="v-settings__link" aria-current={s === section ? "page" : undefined}>
              {S.settings.sections[s].title}
            </a>
          ))}
        </nav>
        <div className="v-screen__body">
          <div className="v-settings__col">
            <Text as="h1" variant="headline-lg">{meta.title}</Text>
            <Text variant="body" muted className="v-home__lede">{meta.lede}</Text>
            {prefs.connected && ["providers", "general", "appearance", "models", "keyboard"].includes(section) && <ScopedPreferences key={project?.id ?? "global"} section={section} agents={agentChoices(providers.list)} />}
            {section === "providers" && (
              <section aria-labelledby="v-settings-agents" className="v-panel v-settings__checks">
                <div className="v-panel__bar">
                  <h2 id="v-settings-agents" className="v-panel__title">{S.providers.installed}</h2>
                </div>
                <div className="v-panel__body v-prov__list">
                  {providers.error && <Problem title={S.providers.listFailed} detail={providers.error} retry={providers.reload} />}
                  {!providers.list && !providers.error && <LoadingRows rows={2} label={S.providers.checking} />}
                  {providers.list?.map((p) => <ProviderCard key={p.id} provider={p} />)}
                </div>
              </section>
            )}
            {section === "providers" && (
              <section aria-labelledby="v-settings-checks" className="v-panel v-settings__checks">
                <div className="v-panel__bar">
                  <h2 id="v-settings-checks" className="v-panel__title">{S.settings.checks}</h2>
                  <span className="v-panel__end"><CheckedAt at={checks.at} /></span>
                </div>
                <div className="v-panel__body">
                  {checks.err && <Problem title={S.settings.failed} detail={checks.err} retry={checks.run} />}
                  {!checks.pre && !checks.err && <LoadingRows rows={4} label={S.states.loading} />}
                  {checks.pre && <PreflightList rows={checks.pre.rows} />}
                </div>
              </section>
            )}
            {section === "keyboard" && (
              <section className="v-panel" aria-label={meta.title}>
                <ul className="v-settings__keys">
                  {SHORTCUTS.map((k) => <li key={k.label}><span>{k.label}</span><Kbd keys={k.label === S.settings.keys.palette ? shortcutKeys(prefs.view?.effective.shortcuts.palette ?? DEFAULT_PREFERENCES.shortcuts.palette) : k.label === S.settings.keys.settings ? shortcutKeys(prefs.view?.effective.shortcuts.settings ?? DEFAULT_PREFERENCES.shortcuts.settings) : k.keys} /></li>)}
                </ul>
              </section>
            )}
            {(section === "notifications" || section === "mobile" || section === "diagnostics") && (
              <p className="v-settings__later">{S.settings.later}</p>
            )}
          </div>
        </div>
      </div>
      {section === "providers" && <ActionBar label={S.settings.barLabel} actions={[{ id: "recheck", label: S.settings.recheck, run: recheck }]} />}
    </div>
  );
}

/** One choice per provider: not installed is shown but cannot be chosen; untested stays selectable (providers.md G2, G5).
 * Before the check answers (or if it cannot), the two adapters this build knows are offered; Start still refuses one that is missing. */
function agentChoices(list: ProviderInfo[] | null): AgentChoice[] {
  if (!list) return [["claude", S.providers.claude], ["codex", S.providers.codex]];
  return list.map((p) => [p.id, !p.installed ? S.providers.notInstalledOption(p.name) : p.tested === "untested" ? S.providers.untestedOption(p.name) : p.name, !p.installed] as const);
}

function ScopedPreferences({ section, agents }: { section: string; agents: AgentChoice[] }) {
  const { project } = useCurrentProject(); const available = preferenceScope(project) ? project : null;
  const [global, setGlobal] = useState(!available); const [dirty, setDirty] = useState(false);
  return <><div className="v-pref-scope"><label>{S.preferences.scope}<select aria-label={S.preferences.scope} disabled={dirty} value={global ? "global" : "project"} onChange={e => setGlobal(e.target.value === "global")}>
    <option value="global">{S.preferences.global}</option>{available && <option value="project">{available.name}</option>}
  </select></label>{dirty && <p>{S.preferences.scopeHint}</p>}</div>
  <PreferenceForm projectId={global ? null : available?.id ?? null} projectName={available?.name} section={section} onDirty={setDirty} agents={agents} /></>;
}
