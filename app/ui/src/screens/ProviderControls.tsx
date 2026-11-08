// FG-09 in the UI: the provider check (`provider_list`), and what each provider can do in a run. A control the provider
// cannot do is shown disabled with its reason, never hidden (providers.md G6). Reading the list runs each CLI's own status
// command, so it is fetched only where a view needs it: Settings > Providers, or a view about a non-claude run.
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Button, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { api } from "../ipc";
import type { ResumeTarget } from "../ipc";
import { parseProviderList } from "../providerContract";
import type { CapabilityFlags, ProviderInfo } from "../providerContract";
import { problemDetail } from "./states";
import "./providers.css";

const C = S.providers;

export type Providers = { list: ProviderInfo[] | null; error: string; loading: boolean; reload: () => void };

/** The provider list, validated against the contract. A late answer from an earlier ask (or after `on` turns off) is dropped. */
export function useProviders(on: boolean): Providers {
  const seq = useRef(0);
  const [s, setS] = useState<Omit<Providers, "reload">>({ list: null, error: "", loading: on });
  const reload = useCallback(() => {
    const mine = ++seq.current;
    setS((x) => ({ ...x, error: "", loading: true }));
    api.providerList().then((v) => {
      if (mine !== seq.current) return;
      const r = parseProviderList(JSON.stringify(v ?? null));
      setS(r.ok ? { list: r.value.providers, error: "", loading: false }
        : { list: null, error: r.reason === "unsupported_version" ? C.updateNeeded : C.unreadable, loading: false });
    }, (e) => {
      if (mine === seq.current) setS({ list: null, error: problemDetail(e), loading: false });
    });
  }, []);
  useEffect(() => {
    if (on) reload();
    return () => { ++seq.current; };
  }, [on, reload]);
  return { ...s, reload };
}

const FLAGS = ["pty_attach", "transcript", "inspect", "message", "redirect", "resume", "vajra_workflow"] as const;
type Row = { key: string; on: boolean; text: string };

/** Each control with its state: on, or off with the provider's own reason (this build's words when it gives none). */
export function controlRows(p: ProviderInfo): Row[] {
  const reason = (k: string) => p.limits.find((l) => l.control === k)?.reason ?? C.offFallback(p.name);
  const caps: CapabilityFlags = p.capabilities;
  const rows: Row[] = FLAGS.map((k) => ({ key: k, on: caps[k], text: caps[k] ? C.on : reason(k) }));
  rows.splice(5, 0, { key: "stop", on: caps.stop !== "other", text: C.stop[caps.stop] ?? C.stop.other });
  // A limit on a control that is not a capability flag (for example `start`) is still shown, off, with its reason.
  for (const l of p.limits) if (!rows.some((r) => r.key === l.control)) rows.push({ key: l.control, on: false, text: l.reason });
  return rows;
}

/** What a run on this provider can do: the controls it has in one line, then each one it lacks as a disabled control with its
 * reason. `only="off"` (the run views and Start) shows just the ones it lacks. */
export function ProviderControls({ provider, only }: { provider: ProviderInfo; only?: "off" }) {
  const id = useId();
  const rows = controlRows(provider);
  const on = rows.filter((r) => r.on);
  const off = rows.filter((r) => !r.on);
  const stop = rows.find((r) => r.key === "stop");
  return (
    <div className="v-prov__caps">
      {!only && on.length > 0 && (
        <p className="v-prov__has">
          {C.has(on.filter((r) => r.key !== "stop").map((r) => C.capability[r.key] ?? r.key))}
          {stop?.on && <> {C.stopSentence(stop.text)}</>}
        </p>
      )}
      {off.length > 0 && (
        <ul className="v-prov__controls" aria-label={C.controls(provider.name)}>
          {off.map((r) => {
            const rid = `${id}-${r.key}`;
            return (
              <li key={r.key}>
                <Button size="sm" variant="ghost" disabled aria-describedby={rid}>{C.capability[r.key] ?? r.key}</Button>
                <span id={rid} className="v-text v-text--body-sm v-text--muted">{r.text}</span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/** One provider in Settings: where it is, its version against the tested range, its login in plain words, what it can do. */
export function ProviderCard({ provider: p }: { provider: ProviderInfo }) {
  const id = useId();
  const tested = C.tested[p.tested] ?? C.tested.unknown;
  return (
    <section className="v-prov" aria-labelledby={id}>
      <h3 id={id} className="v-prov__title">{p.name}</h3>
      <dl className="v-prov__facts">
        <dt>{C.version}</dt>
        <dd>
          {p.installed ? <span className="v-num">{p.version ?? C.noVersion}</span> : <span>{C.loginState.not_installed}</span>}
          {p.installed && <span className="v-prov__tested">{tested}</span>}
          {p.installed && p.tested_range && <Text as="span" variant="body-sm" muted>{C.testedRange(p.tested_range)}</Text>}
        </dd>
        <dt>{C.path}</dt>
        <dd>{p.path ? <code className="v-prov__path">{p.path}</code> : <Text as="span" variant="body-sm" muted>{C.notInstalled}</Text>}</dd>
        {p.installed && <>
          <dt>{C.login}</dt>
          <dd>
            <span>{C.loginState[p.login.state] ?? C.loginState.cannot_tell}</span>
            <Text as="span" variant="body-sm" muted>{p.login.route}</Text>
            {p.login.detail && <Text as="span" variant="body-sm" muted>{p.login.detail}</Text>}
            {p.login.state === "signed_out" && <Text as="span" variant="body-sm" muted>{C.signedOut(p.name)}</Text>}
          </dd>
        </>}
      </dl>
      {p.billing.key_in_app_env && <Text variant="body-sm" muted className="v-prov__key">{C.keyInEnv(p.billing.key_in_app_env, p.name)}</Text>}
      {p.installed && <ProviderControls provider={p} />}
    </section>
  );
}

/** Which provider ran a campaign, from the run index (`resume_targets`); absent reads as claude, as in the core. */
function useRunProvider(slug: string): string | null {
  const [found, setFound] = useState<{ slug: string; provider: string } | null>(null);
  useEffect(() => {
    if (!slug) return;
    let live = true;
    api.resumeTargets().then((r: { targets: Record<string, ResumeTarget> }) => {
      if (live) setFound({ slug, provider: r.targets?.[slug]?.provider ?? "claude" });
    }, () => { /* No index, no strip: the run views carry on without it. */ });
    return () => { live = false; };
  }, [slug]);
  return found && found.slug === slug ? found.provider : null;
}

/** In Run and Crew: a run on a provider other than Claude names it, and lists what it cannot do there, with reasons. */
export function RunProvider({ slug }: { slug: string }) {
  const provider = useRunProvider(slug);
  const other = !!provider && provider !== "claude";
  const { list } = useProviders(other);
  const p = other ? list?.find((x) => x.id === provider) : undefined;
  if (!other || !p) return null;
  return (
    <section className="v-prov__run" aria-label={C.runsOn(p.name)}>
      <Text variant="label">{C.runsOn(p.name)}</Text>
      {!p.installed && <Text variant="body-sm" muted>{C.resumeOff(p.name)}</Text>}
      <ProviderControls provider={p} only="off" />
    </section>
  );
}
