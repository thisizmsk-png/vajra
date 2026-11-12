// Settings > Providers > Set up (ADR-17 step 1): each agent CLI the catalog knows, its state, the vendor's own steps for
// one that is missing or broken, and the CLI's own sign-in in a terminal tab. This step installs nothing, so there is
// no Install button: one-click install comes after a security review of each catalog source.
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Button, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api } from "../../ipc";
import { parseSetupList } from "../../setupContract";
import type { SetupList, SetupRow } from "../../setupContract";
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

function Steps({ row }: { row: SetupRow }) {
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

function Row({ row }: { row: SetupRow }) {
  const id = useId();
  const installed = row.state === "installed" || row.state === "unsupported";
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
        {list?.rows.map((r) => <Row key={r.id} row={r} />)}
        {list && <Engine engine={list.engine} />}
      </div>
    </section>
  );
}
