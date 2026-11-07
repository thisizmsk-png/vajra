import { useCallback, useEffect, useState } from "react";
import { Skeleton, Text } from "../../../components/ui/base";
import { ActionBar } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { Preflight } from "../../ipc";
import { useCommands } from "../../palette";
import { PreflightList } from "../PreflightList";

/** The app has no preferences yet; Settings shows the environment checks and where each failing one is fixed. */
export function Settings() {
  const [pre, setPre] = useState<Preflight | null>(null);
  const [err, setErr] = useState("");
  const run = useCallback(() => {
    api.preflight().then((p) => { setPre(p); setErr(""); }, (e) => setErr(errorText(e)));
  }, []);
  useEffect(run, [run]);
  useCommands([{ id: "recheck", label: S.settings.recheck, run }]);
  return (
    <div className="v-screen v-settings">
      <div className="v-screen__body">
        <div className="v-home__col">
          <Text as="h1" variant="headline-lg">{S.settings.title}</Text>
          <Text variant="body" muted className="v-home__lede">{S.settings.lede}</Text>
          <section aria-labelledby="v-settings-checks" className="v-settings__checks">
            <h2 id="v-settings-checks" className="v-text v-text--headline-md">{S.settings.checks}</h2>
            {err && <div role="alert"><Text variant="body-sm">{S.settings.failed} {err}</Text></div>}
            {!pre && !err && <Skeleton />}
            {pre && <PreflightList rows={pre.rows} />}
          </section>
        </div>
      </div>
      <ActionBar label={S.settings.barLabel} actions={[{ id: "recheck", label: S.settings.recheck, run }]} />
    </div>
  );
}
