import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Input, Section, SegmentedControl, StatusDot, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { Campaign, Preflight, ResumeTarget } from "../../ipc";
import { useRoute } from "../../router";
import { toInput, validate } from "./validate";
import type { Draft } from "./validate";

type Load<T> = { state: "loading" } | { state: "ok"; data: T } | { state: "error"; text: string };

function useLoad<T>(fetch: () => Promise<T>, refreshOnChange = false): [Load<T>, () => void] {
  const [v, setV] = useState<Load<T>>({ state: "loading" });
  const run = useCallback(() => {
    fetch().then((data) => setV({ state: "ok", data }), (e) => setV({ state: "error", text: errorText(e) }));
  }, [fetch]);
  useEffect(run, [run]);
  useEffect(() => (refreshOnChange ? onChanged(run) : undefined), [refreshOnChange, run]);
  return [v, run];
}

const fetchPreflight = () => api.preflight();
const fetchCampaigns = () => api.listCampaigns().then((r) => r.campaigns);
const fetchTargets = () => api.resumeTargets().then((r) => r.targets);

export function Home() {
  const [, go] = useRoute();
  const [pre, recheck] = useLoad<Preflight>(fetchPreflight);
  const [camps] = useLoad<Campaign[]>(fetchCampaigns, true);
  const [targets] = useLoad<Record<string, ResumeTarget>>(fetchTargets, true);
  const [d, setD] = useState<Draft>({ repo: "", ask: "", base: "HEAD", rounds: "3", maxInFlight: "4" });
  const [shown, setShown] = useState(false); // show field errors only after the first try
  const [busy, setBusy] = useState<string | null>(null);
  const [fail, setFail] = useState("");
  const set = (k: keyof Draft) => (v: string) => setD((x) => ({ ...x, [k]: v }));
  const errs = validate(d);
  const ready = pre.state === "ok" && pre.data.start_enabled;

  const launch = async (key: string, begin: () => Promise<{ campaign: string; run_id: string; stream: string }>) => {
    setBusy(key);
    setFail("");
    try {
      const r = await begin();
      go("run", { slug: r.campaign, id: r.run_id, stream: r.stream });
    } catch (e) {
      setFail(errorText(e));
      setBusy(null);
    }
  };

  const start = () => {
    setShown(true);
    if (Object.keys(errs).length === 0 && ready) void launch("start", () => api.startRun(toInput(d), d.repo.trim()));
  };
  // Resume continues the session that ran the campaign (claude --resume), in the directory it ran in. Only a campaign
  // this app started has a recorded session; any other cannot be resumed from here.
  const targetOf = (c: Campaign) => (targets.state === "ok" ? targets.data[c.slug] : undefined);
  const resume = (c: Campaign) => {
    const t = targetOf(c);
    if (t) void launch(c.slug, () => api.resumeRun(t.session_id, c.slug, t.cwd));
  };
  const err = (k: keyof Draft) => (shown ? errs[k] : undefined);

  return (
    <div className="v-home">
      <Text as="h1" variant="headline-lg">{S.home.title}</Text>

      <Section title={S.home.preflight} action={<Button size="sm" onClick={recheck}>{S.home.preflightRetry}</Button>}>
        {pre.state === "loading" && <Text variant="body-sm" muted>…</Text>}
        {pre.state === "error" && <div role="alert"><Text variant="body-sm">{S.home.preflightFailed} {pre.text}</Text></div>}
        {pre.state === "ok" && (
          <ul className="v-home__checks">
            {pre.data.rows.map((r) => (
              <li key={r.name}>
                <StatusDot status={r.ok ? "pass" : "fail"} />
                <span>{r.name}</span>
                <Text variant="body-sm" muted>{r.detail}</Text>
                {!r.ok && r.fix && <code className="v-home__fix">{r.fix}</code>}
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section title={S.home.start}>
        <form className="v-home__form" onSubmit={(e) => { e.preventDefault(); start(); }} noValidate>
          <Input label={S.home.repo} hint={S.home.repoHint} value={d.repo} onChange={set("repo")} error={err("repo")} />
          <Input type="textarea" rows={4} label={S.home.ask} value={d.ask} onChange={set("ask")} error={err("ask")} />
          <Input label={S.home.base} hint={S.home.baseHint} value={d.base} onChange={set("base")} error={err("base")} />
          <SegmentedControl label={S.home.rounds} value={d.rounds} onChange={set("rounds")}
            options={[{ value: "1", label: "1" }, { value: "2", label: "2" }, { value: "3", label: "3" }]} />
          <details>
            <summary>{S.home.advanced}</summary>
            <Input type="number" label={S.home.maxInFlight} value={d.maxInFlight} onChange={set("maxInFlight")} error={err("maxInFlight")} />
          </details>
          {fail && <div role="alert"><Text variant="body-sm">{fail}</Text></div>}
          <Button variant="primary" onClick={start} disabled={!ready} busy={busy === "start"}>
            {busy === "start" ? S.home.starting : S.home.startButton}
          </Button>
        </form>
      </Section>

      <Section title={S.home.campaigns}>
        {camps.state === "error" && <div role="alert"><Text variant="body-sm">{S.home.campaignsFailed} {camps.text}</Text></div>}
        {camps.state === "ok" && camps.data.length === 0 && <Text variant="body-sm" muted>{S.home.campaignsEmpty}</Text>}
        {camps.state === "ok" && camps.data.length > 0 && (
          <>
            <ul className="v-home__list" aria-label={S.home.campaigns}>
              {camps.data.map((c) => (
                <li key={c.slug}>
                  <span>{c.slug}{c.goal ? ` · ${c.goal}` : ""}</span>
                  {c.error ? <Badge status="danger">{c.error}</Badge> : (
                    <>
                      {c.total != null && <Text variant="body-sm" numeric>{S.home.passing(c.passing ?? 0, c.total)}</Text>}
                      {targetOf(c) ? (
                        <Button size="sm" onClick={() => resume(c)} busy={busy === c.slug}>
                          {busy === c.slug ? S.home.resuming : S.home.resume}
                        </Button>
                      ) : (
                        <Text variant="body-sm" muted>{S.home.resumeElsewhere}</Text>
                      )}
                    </>
                  )}
                </li>
              ))}
            </ul>
          </>
        )}
      </Section>
    </div>
  );
}
