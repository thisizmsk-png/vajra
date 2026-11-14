// Settings > Providers > Set up (ADR-17): each agent CLI the catalog knows, its state, the vendor's own steps for one
// that is missing or broken, and the CLI's own sign-in in a terminal tab. A source that passed its security review
// (`one_click`) also gets the one-click sheet; every other one stays "Install it yourself", with no Install button.
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Button, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api } from "../../ipc";
import { parseSetupList, parseSetupPlan, parseSetupRun } from "../../setupContract";
import type { RunReport, SetupList, SetupPlan, SetupRow } from "../../setupContract";
import { LoadingRows, Problem, problemDetail } from "../states";
import { ProviderIcon } from "../ProviderIcon";
import { openSignIn } from "../../terminal/TerminalDock";

const C = S.setup;


/** The Set up list. Asking for it also re-resolves the user PATH in the core (ADR-17 rule 7), so a page recheck awaits
 * `reload` before the checks that read the PATH. The returned promise never rejects. `settled` turns true once the first
 * answer (or error) since `on` arrives, so the page can hold its PATH-dependent reads until then. */
export function useSetup(on = true) {
  const seq = useRef(0);
  const [s, setS] = useState<{ list: SetupList | null; error: string }>({ list: null, error: "" });
  const [settled, setSettled] = useState(false);
  const reload = useCallback((): Promise<void> => {
    const mine = ++seq.current;
    setS((x) => ({ ...x, error: "" }));
    return api.setupList().then((v) => {
      if (mine !== seq.current) return;
      const r = parseSetupList(v);
      setS(r.ok ? { list: r.value, error: "" } : { list: null, error: r.reason === "unsupported_version" ? C.updateNeeded : C.unreadable });
      setSettled(true);
    }, (e) => {
      if (mine !== seq.current) return;
      setS({ list: null, error: problemDetail(e) });
      setSettled(true);
    });
  }, []);
  useEffect(() => {
    if (on) reload();
    else setSettled(false);
    return () => { ++seq.current; };
  }, [on, reload]);
  return { ...s, settled, reload };
}

export function Steps({ row }: { row: SetupRow }) {
  return (
    <div className="v-setup__steps">
      {row.reason_code && C.reason[row.reason_code] && <Text variant="body-sm" muted>{C.reason[row.reason_code]}</Text>}
      <ul className="v-setup__cmds">
        {row.commands.map((c) => (
          <li key={c.command}>
            <span className="v-setup__tool">{C.tool[c.tool] ?? c.tool}</span>
            <code className="v-prov__path">{c.command}</code>
          </li>
        ))}
      </ul>
      <p className="v-setup__docs">
        <span className="v-setup__tool" aria-hidden="true">{C.docs}</span>
        <code className="v-prov__path" aria-label={C.vendorSteps(row.name)}>{row.docs_url}</code>
      </p>
    </div>
  );
}

type Sheet =
  | { at: "idle" }
  | { at: "planning" }
  | { at: "error"; detail: string }
  | { at: "manual"; code: string }
  | { at: "plan"; plan: SetupPlan }
  | { at: "running"; plan: SetupPlan }
  | { at: "done"; plan: SetupPlan; report: RunReport };

const MAX_OUTPUT = 400;

/** The live install output: the `setup-<id>` stream, read while the run is in flight. */
function useOutput(stream: string, on: boolean) {
  const [lines, setLines] = useState<string[]>([]);
  const after = useRef(0);
  const pull = useCallback(() => api.tail(stream, after.current).then((t) => {
    if (!t?.lines?.length) return;
    after.current = t.lines[t.lines.length - 1].seq;
    setLines((l) => [...l, ...t.lines.map((x) => x.text)].slice(-MAX_OUTPUT));
  }, () => undefined), [stream]);
  useEffect(() => {
    if (!on) return;
    const t = setInterval(pull, 400);
    return () => clearInterval(t);
  }, [on, pull]);
  return { lines, pull };
}

/** ADR-17 17b: the one-click sheet. Shows the exact command and everything it binds; Install asks the core for a
 * consent token on that plan's hash and runs with it. The token lives only in this click's local. */
export function OneClick({ row, reload, opened }: { row: SetupRow; reload: () => Promise<void>; opened: () => void }) {
  const [s, setS] = useState<Sheet>({ at: "idle" });
  const titleId = useId();
  const out = useOutput(`setup-${row.id}`, s.at === "running");
  const plan = () => {
    opened();
    setS({ at: "planning" });
    api.setupPlan(row.id).then((v) => {
      const r = parseSetupPlan(v);
      if (!r.ok) setS({ at: "error", detail: r.reason === "unsupported_version" ? C.updateNeeded : C.unreadable });
      else setS(r.value.kind === "plan" ? { at: "plan", plan: r.value.plan } : { at: "manual", code: r.value.code });
    }, (e) => setS({ at: "error", detail: problemDetail(e) }));
  };
  const install = async (p: SetupPlan) => {
    setS({ at: "running", plan: p });
    try {
      const { token } = await api.setupConsent(row.id, p.hash);
      const report = parseSetupRun(await api.setupRun(row.id, token));
      await out.pull();
      setS(report ? { at: "done", plan: p, report } : { at: "error", detail: C.unreadable });
    } catch (e) {
      setS({ at: "error", detail: problemDetail(e) });
    }
    await reload();
  };
  if (s.at === "idle") {
    return <div className="v-setup__actions"><Button size="sm" onClick={plan}>{C.planOneClick(row.name)}</Button></div>;
  }
  if (s.at === "planning") return <LoadingRows rows={2} label={C.planning} />;
  if (s.at === "error") return <Problem title={C.planFailed} detail={s.detail} retry={plan} />;
  if (s.at === "manual") return <Text variant="body-sm" muted>{C.manual[s.code] ?? C.manualOther}</Text>;
  const p = s.plan;
  const verb = row.state === "broken" ? C.repair(row.name) : C.install(row.name);
  return (
    <section className="v-setup__sheet" aria-labelledby={titleId}>
      <h4 id={titleId} className="v-prov__title">{C.sheetTitle(row.name)}</h4>
      <dl className="v-prov__facts">
        <dt>{C.field.command}</dt><dd><code className="v-prov__path">{p.command.join(" ")}</code></dd>
        <dt>{C.field.source}</dt><dd>{p.source}</dd>
        {p.version && <><dt>{C.field.version}</dt><dd><span className="v-num">{p.version}</span></dd></>}
        <dt>{C.field.tools}</dt>
        <dd>
          <ul className="v-setup__cmds">
            {p.tools.map((t) => (
              <li key={t.role}>
                <span className="v-setup__tool">{t.role}</span>
                <code className="v-prov__path">{t.path}</code>
                {t.owned_by_you && <Text as="span" variant="body-sm" muted>{C.ownedByYou}</Text>}
              </li>
            ))}
          </ul>
        </dd>
        {p.target && <><dt>{C.field.target}</dt><dd><code className="v-prov__path">{p.target}</code></dd></>}
        {p.scope_registry && <><dt>{C.field.scope}</dt><dd><code className="v-prov__path">{p.scope_registry}</code></dd></>}
        {p.deps.length > 0 && <><dt>{C.field.deps}</dt><dd>{p.deps.join(", ")}</dd></>}
        {p.needs_admin && <><dt>{C.field.admin}</dt><dd><code className="v-prov__path">{p.needs_admin.path}</code></dd></>}
      </dl>
      {p.notes.map((n) => C.note[n] && <Text key={n} variant="body-sm" muted>{C.note[n]}</Text>)}
      {p.needs_admin && <Text variant="body-sm">{C.admin[p.needs_admin.reason] ?? C.admin.not_writable}</Text>}
      {s.at === "plan" && !p.needs_admin && (
        <div className="v-setup__actions">
          <Button size="sm" onClick={() => install(p)}>{verb}</Button>
          <Text as="span" variant="body-sm" muted>{C.consentHint}</Text>
        </div>
      )}
      {s.at === "running" && <p className="v-setup__status" aria-live="polite"><Text as="span" variant="body-sm" muted>{C.running}</Text></p>}
      {(s.at === "running" || s.at === "done") && out.lines.length > 0 && (
        <pre className="v-setup__output" aria-label={C.output(row.name)}>{out.lines.join("\n")}</pre>
      )}
      {s.at === "done" && <p className="v-setup__status" role="status"><Text as="span" variant="body-sm">{C.outcome[s.report.outcome] ?? C.outcome.broken}</Text></p>}
    </section>
  );
}

function Row({ row, reload }: { row: SetupRow; reload: () => Promise<void> }) {
  const id = useId();
  const installed = row.state === "installed" || row.state === "unsupported";
  // Once opened, the sheet stays: a successful install turns the row Installed, which is no longer offered one-click,
  // and the report and output must not vanish with it.
  const [opened, setOpened] = useState(false);
  const open = useCallback(() => setOpened(true), []);
  return (
    <section className="v-prov v-setup__row" aria-labelledby={id}>
      <h3 id={id} className="v-prov__title"><ProviderIcon family={row.id} />{row.name}</h3>
      <dl className="v-prov__facts">
        <dt>{S.providers.version}</dt>
        <dd>
          <span className="v-setup__state" data-state={row.state}>{C.state[row.state]}</span>
          {row.version && <span className="v-num">{row.version}</span>}
        </dd>
        {row.path && <><dt>{S.providers.path}</dt><dd><code className="v-prov__path">{row.path}</code></dd></>}
      </dl>
      {!installed && <Steps row={row} />}
      {(row.one_click || opened) && <OneClick row={row} reload={reload} opened={open} />}
      {row.can_sign_in && (
        <div className="v-setup__actions">
          <Button size="sm" variant="ghost" onClick={() => openSignIn({ id: row.id, label: C.signInTab(row.name) })}>{C.signIn(row.name)}</Button>
          <Text as="span" variant="body-sm" muted>{C.signInHint}</Text>
        </div>
      )}
    </section>
  );
}

function Engine({ engine }: { engine: SetupList["engine"] }) {
  const id = useId();
  return (
    <section className="v-prov v-setup__row" aria-labelledby={id}>
      <h3 id={id} className="v-prov__title">{C.engine}</h3>
      <dl className="v-prov__facts">
        <dt>{S.providers.version}</dt>
        <dd><span className="v-setup__state" data-state={engine.installed ? "installed" : "manual"}>{engine.installed ? C.engineState.installed : C.engineState.missing}</span></dd>
      </dl>
      {!engine.installed && (
        <div className="v-setup__steps">
          <Text variant="body-sm" muted>{C.engineHint}</Text>
          <code className="v-prov__path">{engine.command}</code>
        </div>
      )}
    </section>
  );
}

/** The Set up panel; Settings > Providers mounts it above the agents on this Mac and passes its own `useSetup`, so the
 * page's Check again re-reads it. Without one the panel loads by itself. */
export function SetupPanel({ setup }: { setup?: ReturnType<typeof useSetup> }) {
  const own = useSetup(!setup);
  const { list, error, reload } = setup ?? own;
  return (
    <section aria-labelledby="v-settings-setup" className="v-panel v-settings__checks">
      <div className="v-panel__bar">
        <h2 id="v-settings-setup" className="v-panel__title">{C.title}</h2>
      </div>
      <div className="v-panel__body v-prov__list">
        <Text variant="body-sm" muted>{C.lede}</Text>
        {error && <Problem title={C.failed} detail={error} retry={reload} />}
        {!list && !error && <LoadingRows rows={3} label={C.checking} />}
        {list?.rows.map((r) => <Row key={r.id} row={r} reload={reload} />)}
        {list && <Engine engine={list.engine} />}
      </div>
    </section>
  );
}
