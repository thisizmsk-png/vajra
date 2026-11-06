import { useCallback, useEffect, useState } from "react";
import { Button, Section, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { Preflight } from "../../ipc";
import { PreflightList } from "../PreflightList";

/** The app has no preferences yet; Settings shows the environment checks and where each failing one is fixed. */
export function Settings() {
  const [pre, setPre] = useState<Preflight | null>(null);
  const [err, setErr] = useState("");
  const run = useCallback(() => {
    api.preflight().then((p) => { setPre(p); setErr(""); }, (e) => setErr(errorText(e)));
  }, []);
  useEffect(run, [run]);
  return (
    <div className="v-home">
      <Text as="h1" variant="headline-lg">{S.settings.title}</Text>
      <Section title={S.settings.checks} action={<Button size="sm" onClick={run}>{S.settings.recheck}</Button>}>
        {err && <div role="alert"><Text variant="body-sm">{S.settings.failed} {err}</Text></div>}
        {pre && <PreflightList rows={pre.rows} />}
      </Section>
    </div>
  );
}
