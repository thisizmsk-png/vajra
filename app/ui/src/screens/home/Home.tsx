import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, Input, Kbd, SegmentedControl, StatusDot, Text, ariaShortcut } from "../../../components/ui/base";
import { ActionBar } from "../../../components/ui/desktop";
import type { StatusKey } from "../../../strings";
import { S } from "../../../strings";
import { api, onChanged } from "../../ipc";
import { LoadingRows, Problem, problemDetail, CheckedAt } from "../states";
import type { Campaign, Preflight, ResumeTarget } from "../../ipc";
import { usePreferences } from "../../preferences";
import { useCommands } from "../../palette";
import { useRoute } from "../../router";
import { useCurrentProject } from "../../projects/current";
import { PreflightList } from "../PreflightList";
import { toInput, validate } from "./validate";
import type { Draft } from "./validate";

type Load<T> = { state: "loading" } | { state: "ok"; data: T } | { state: "error"; text: string };

function useLoad<T>(fetch: () => Promise<T>, refreshOnChange = false): [Load<T>, () => void, Date | null] {
  const [v, setV] = useState<Load<T>>({ state: "loading" });
  const [at, setAt] = useState<Date | null>(null);
  const run = useCallback(() => {
    setAt(null);
    fetch().then((data) => setV({ state: "ok", data }), (e) => setV({ state: "error", text: problemDetail(e) }))
      .finally(() => setAt(new Date()));
  }, [fetch]);
  useEffect(run, [run]);
  useEffect(() => (refreshOnChange ? onChanged(run) : undefined), [refreshOnChange, run]);
  return [v, run, at];
}

const fetchPreflight = () => api.preflight();
const START_KEYS = ["⌘", "↵"];

/** A campaign's status as a status shape; anything unrecognised is shown as unknown, never as done. */
const CAMPAIGN_STATUS: Record<string, StatusKey> = { active: "in_progress", running: "in_progress", done: "done", delivered: "done", stopped: "blocked", failed: "failed" };
const campaignStatus = (c: Campaign): StatusKey => CAMPAIGN_STATUS[c.status ?? ""] ?? "unknown";
const basename = (p: string) => p.trim().replace(/\/+$/, "").split("/").pop() ?? "";
const fetchCampaigns = () => api.listCampaigns().then((r) => r.campaigns);
const fetchTargets = () => api.resumeTargets().then((r) => r.targets);

export function Home() {
  const [, go] = useRoute();
  const prefs = usePreferences();
  // Inside a project the folder is the project's, and the list is the project's own campaigns (FG-03).
  const { project } = useCurrentProject();
  const [pre, recheck, checkedAt] = useLoad<Preflight>(fetchPreflight);
  const [allCamps, reloadCamps] = useLoad<Campaign[]>(fetchCampaigns, true);
  const [targets, reloadTargets] = useLoad<Record<string, ResumeTarget>>(fetchTargets, true);
  // Inside a project the folders decide which campaigns are its own. Until they are read the list is loading, and if
  // they cannot be read it says so: filtering without them would empty every project and put everything in Unsorted.
  const inProject = (t: Record<string, ResumeTarget>) => (c: Campaign) => {
    const root = t[c.slug]?.cwd?.trim().replace(/\/+$/, "");
    return project?.bucket ? !root : root === project?.root;
  };
  const camps: Load<Campaign[]> = allCamps.state !== "ok" ? allCamps
    : !project ? allCamps
    : targets.state === "ok" ? { state: "ok", data: allCamps.data.filter(inProject(targets.data)) }
    : targets;
  const retryCamps = allCamps.state === "error" ? reloadCamps : reloadTargets;
  const campsFailed = allCamps.state === "error" ? S.home.campaignsFailed : S.home.targetsFailed;
  const [d, setD] = useState<Draft>({ repo: project?.root ?? "", ask: "", base: "HEAD", rounds: "3", maxInFlight: "4" });
  const defaultsApplied = useRef(false);
  useEffect(() => {
    if (!prefs.view || defaultsApplied.current) return;
    defaultsApplied.current = true;
    setD(x => ({ ...x, rounds: String(prefs.view!.effective.rounds), maxInFlight: String(prefs.view!.effective.workers) }));
  }, [prefs.view]);
  const [shown, setShown] = useState(false); // show field errors only after the first try
  const [busy, setBusy] = useState<string | null>(null);
  const [fail, setFail] = useState("");
  const set = (k: keyof Draft) => (v: string) => { if (k === "rounds" || k === "maxInFlight") defaultsApplied.current = true; setD((x) => ({ ...x, [k]: v })); };
  const errs = validate(d);
  const unsupported = !!prefs.view && (prefs.view.effective.agent !== "claude" || prefs.view.effective.model !== "");
  const ready = pre.state === "ok" && pre.data.start_enabled && (!prefs.connected || !!prefs.view) && !unsupported;

  // A ref, not the busy state: a second Cmd+Return in the same tick still sees busy as null.
  const inFlight = useRef(false);
  const launch = async (key: string, begin: () => Promise<{ campaign: string; run_id: string; stream: string }>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(key);
    setFail("");
    try {
      const r = await begin();
      go("run", { slug: r.campaign, id: r.run_id, stream: r.stream });
    } catch (e) {
      setFail(problemDetail(e));
      setBusy(null);
      inFlight.current = false;
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

  useCommands([{ id: "recheck", label: S.home.preflightRetry, run: recheck }]);

  const failing = pre.state === "ok" ? pre.data.rows.filter((r) => !r.ok).length : 0;
  const repoName = basename(d.repo);

  return (
    <div className="v-screen v-home">
      <div className="v-screen__body">
        <div className="v-home__col">
          <Text as="h1" variant="headline-lg">{S.home.start}</Text>
          <Text variant="body" muted className="v-home__lede">{S.home.lede}</Text>

          {prefs.error && <p role="alert">{prefs.error}</p>}
          {unsupported && <p role="alert">The saved agent or model needs a supported adapter. Choose the native CLI default in Settings before starting.</p>}
          <form className="v-launch" aria-label={S.home.start} noValidate
            onSubmit={(e) => { e.preventDefault(); start(); }}
            onKeyDown={(e) => { if (e.metaKey && !e.shiftKey && !e.altKey && !e.ctrlKey && e.key === "Enter" && !e.nativeEvent.isComposing) { e.preventDefault(); if (!e.repeat) start(); } }}>
            <div className="v-launch__ask">
              <Input type="textarea" rows={3} label={S.home.ask} placeholder={S.home.ask} value={d.ask} onChange={set("ask")} error={err("ask")} />
            </div>
            <div className="v-launch__row">
              <div className="v-launch__repo"><Input label={S.home.repo} hint={S.home.repoHint} value={d.repo} onChange={set("repo")} error={err("repo")} /></div>
              <div className="v-launch__base"><Input label={S.home.base} hint={S.home.baseHint} value={d.base} onChange={set("base")} error={err("base")} /></div>
              <div className="v-launch__rounds">
                {/* The fieldset's legend names the group for screen readers; this is the same word for the eye. */}
                <span className="v-launch__cap" aria-hidden="true">{S.home.rounds}</span>
                <SegmentedControl label={S.home.rounds} value={d.rounds} onChange={set("rounds")}
                  options={[{ value: "1", label: "1" }, { value: "2", label: "2" }, { value: "3", label: "3" }]} />
              </div>
            </div>
            <div className="v-launch__foot">
              <details className="v-launch__adv">
                <summary>{S.home.advanced}</summary>
                <Input type="number" label={S.home.maxInFlight} value={d.maxInFlight} onChange={set("maxInFlight")} error={err("maxInFlight")} />
              </details>
              <Button variant="primary" onClick={start} disabled={!ready} busy={busy === "start"} aria-keyshortcuts={ariaShortcut(START_KEYS)}>
                {busy === "start" ? S.home.starting : S.home.startButton}
                <Kbd keys={START_KEYS} decorative />
              </Button>
            </div>
            {fail && <div role="alert" className="v-launch__fail"><Text variant="body-sm">{fail}</Text></div>}
          </form>

          <section className="v-home__checks" aria-label={S.home.preflight}>
            {pre.state === "loading" && <LoadingRows rows={1} label={S.states.loading} />}
            {pre.state === "error" && <Problem title={S.home.preflightFailed} detail={pre.text} retry={recheck} />}
            {pre.state === "ok" && (
              <details open={failing > 0}>
                <summary className="v-home__checksum">
                  <StatusDot status={!pre.data.start_enabled ? "fail" : failing ? "blocked" : "pass"} label={S.home.checksSummary(pre.data.rows.length - failing, pre.data.rows.length)} />
                  <span className="v-home__checksall">{S.home.checksAll}</span>
                </summary>
                <PreflightList rows={pre.data.rows} />
              </details>
            )}
            <CheckedAt at={checkedAt} />
          </section>

          <section className="v-panel v-home__runs" aria-labelledby="v-home-camps">
            <div className="v-panel__bar"><h2 id="v-home-camps" className="v-panel__title">{S.home.campaigns}</h2></div>
            {camps.state !== "ok" || camps.data.length === 0 ? (
              <div className="v-panel__body">
                {camps.state === "loading" && <LoadingRows label={S.states.loading} />}
                {camps.state === "error" && <Problem title={campsFailed} detail={camps.text} retry={retryCamps} />}
                {camps.state === "ok" && <Text variant="body-sm" muted>{S.home.campaignsEmpty}</Text>}
              </div>
            ) : null}
            {camps.state === "ok" && camps.data.length > 0 && (
              <ul className="v-home__campaigns" aria-label={S.home.campaigns}>
                {camps.data.map((c) => (
                  <li key={c.slug}>
                    <StatusDot status={c.error ? "failed" : campaignStatus(c)} className="v-home__dot" />
                    <span className="v-home__goal">{c.goal || c.slug}</span>
                    <span className="v-home__slug">{c.slug}</span>
                    <span className="v-home__side">
                      {c.error ? <Badge status="danger">{c.error}</Badge> : (
                        <>
                          {c.total != null && <Text variant="body-sm" numeric>{S.home.passing(c.passing ?? 0, c.total)}</Text>}
                          {targetOf(c) ? (
                            <Button size="sm" onClick={() => resume(c)} busy={busy === c.slug}>
                              {busy === c.slug ? S.home.resuming : S.home.resume}
                            </Button>
                          ) : (
                            // A quiet tag on the row; the sentence says what it means to a screen reader and on hover.
                            <span className="v-home__elsewhere" title={S.home.resumeElsewhere}><span aria-hidden="true">{S.home.resumeElsewhereShort}</span><span className="v-sr">{S.home.resumeElsewhere}</span></span>
                          )}
                        </>
                      )}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      </div>
      <ActionBar label={S.home.barLabel}
        context={repoName ? <><b>{repoName}</b><span>{d.repo.trim()}</span></> : <span>{S.home.barNoRepo}</span>}
        actions={[{ id: "recheck", label: S.home.preflightRetry, run: recheck }]} />
    </div>
  );
}
