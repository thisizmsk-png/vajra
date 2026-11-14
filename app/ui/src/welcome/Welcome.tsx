// First-run welcome (option A): one checklist, then the first run. It replaces the empty "No projects yet" screen while setup is
// unfinished. Every read happens once when it opens and again when you come back to the window or a sign-in tab closes; nothing
// polls and nothing writes until you click (a folder pick adds a project, Start run starts the run).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Input, Kbd, Text } from "../../components/ui/base";
import { ActionBar } from "../../components/ui/desktop";
import { S } from "../../strings";
import { api, newActionId } from "../ipc";
import type { Preflight } from "../ipc";
import { usePreferences } from "../preferences";
import { parseProviderList } from "../providerContract";
import type { ProviderInfo } from "../providerContract";
import { useRoute } from "../router";
import { toInput } from "../screens/home/validate";
import { useSetup } from "../screens/settings/SetupPanel";
import { onSignInEnded } from "../terminal/TerminalDock";
import { AgentBody, EngineBody, FolderBody } from "./Bodies";
import { CheckList, CheckRow } from "./CheckRow";
import { derive, reason } from "./model";
import type { AgentInfo, Picked, Read, Repo, Repos, Tools } from "./model";
import "./welcome.css";

const C = S.welcome;
const START_KEYS = ["⌘", "↵"];

/** git and Python 3 are what Apple's developer tools bring; any other failing check is the Start screen's business. */
export function toolsOf(pre: Preflight | null): Tools {
  if (!pre) return "unknown";
  const rows = pre.rows.filter((r) => r.name === "git" || r.name === "python3");
  return rows.length > 0 && rows.some((r) => !r.ok) ? "missing" : "ok";
}

export function Welcome({ onAdded, onStarted }: { onAdded: () => void; onStarted: () => void }) {
  const [, go] = useRoute();
  const prefs = usePreferences();
  const [repos, setRepos] = useState<Repos>({ at: "loading" });
  const [picked, setPicked] = useState<Picked>(null);
  const [pre, setPre] = useState<Preflight | null>(null);
  const setup = useSetup(true);
  const [provs, setProvs] = useState<{ list: ProviderInfo[] | null; failed: boolean; done: boolean }>({ list: null, failed: false, done: false });
  const [addBusy, setAddBusy] = useState(false);
  const [addErr, setAddErr] = useState("");
  const [manual, setManual] = useState<{ at: number; id: number | null } | null>(null);
  const [ask, setAsk] = useState("");
  const [askErr, setAskErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [fail, setFail] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const inFlight = useRef(false);
  const rechecking = useRef(false);
  const alive = useRef(true);
  // Set again on mount: StrictMode runs the cleanup once before the real mount.
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  const scan = useCallback(() => {
    setRepos({ at: "loading" });
    api.projectDiscover().then(
      (r) => alive.current && setRepos(Array.isArray(r?.found) && r.found.length > 0 ? { at: "found", list: r.found, truncated: r.truncated === true } : { at: "none" }),
      (e) => alive.current && setRepos({ at: "failed", detail: reason(e) }),
    );
  }, []);
  const readProviders = useCallback(() => {
    api.providerList().then((v) => {
      if (!alive.current) return;
      const r = parseProviderList(JSON.stringify(v ?? null));
      setProvs(r.ok ? { list: r.value.providers, failed: false, done: true } : { list: null, failed: true, done: true });
    }, () => alive.current && setProvs({ list: null, failed: true, done: true }));
  }, []);
  const readChecks = useCallback(() => {
    api.preflight().then((p) => alive.current && setPre(p), () => alive.current && setPre(null));
  }, []);
  const { reload: reloadSetup } = setup;
  const recheck = useCallback(async () => {
    if (rechecking.current) return;
    rechecking.current = true;
    readChecks();
    readProviders();
    await reloadSetup();
    rechecking.current = false;
  }, [readChecks, readProviders, reloadSetup]);

  useEffect(() => { scan(); readChecks(); readProviders(); }, [scan, readChecks, readProviders]);

  const agents: Read<AgentInfo[]> = useMemo(() => {
    if (setup.error) return "failed";
    if (!setup.list || !provs.done) return "loading";
    return setup.list.rows.map((row) => ({ row, login: provs.list?.find((p) => p.id === row.id)?.login.state ?? "cannot_tell" }));
  }, [setup.list, setup.error, provs]);
  const engineRead: Read<boolean> = setup.error ? "failed" : setup.list ? setup.list.engine.installed : "loading";
  const tools = toolsOf(pre);
  const m = derive({ repos, picked, tools, agents, engine: engineRead });

  // Coming back to the window, or a sign-in tab closing, is when a check can have changed. An event, not a timer.
  const settled = m.ready === 3;
  useEffect(() => {
    if (settled) return;
    const again = () => void recheck();
    window.addEventListener("focus", again);
    const off = onSignInEnded(again);
    return () => { window.removeEventListener("focus", again); off(); };
  }, [settled, recheck]);

  const open = manual && manual.at === m.first ? manual.id : m.first;
  const toggle = (n: number) => setManual({ at: m.first, id: open === n ? null : n });

  // Focus follows the work: the first repository, then the ask. It never takes focus from a control the person is using.
  const focusKey = `${m.first}:${repos.at}:${typeof agents === "string" ? agents : "ok"}:${engineRead === "loading" ? 0 : 1}`;
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const a = document.activeElement;
    if (a && a !== document.body && el.contains(a) && !a.classList.contains("v-chk__head")) return;
    if (a && a !== document.body && !el.contains(a) && a.isConnected) return;
    const target = m.first === 4 ? el.querySelector<HTMLElement>(".v-wel__ask textarea") : el.querySelector<HTMLElement>(".v-chk[data-open] [data-autofocus]");
    target?.focus();
  }, [focusKey, m.first]);

  const use = async (path: string, others: Repo[]) => {
    setAddBusy(true);
    setAddErr("");
    try {
      const r = await api.projectAdd(path);
      setPicked({ id: r.project.id, name: r.project.name, root: r.project.root ?? path });
      let bad = 0;
      for (const o of others) {
        try { await api.projectAdd(o.root); } catch { bad++; }
      }
      if (bad > 0) setAddErr(S.frame.welcome.partial(others.length - bad + 1, bad));
      onAdded();
    } catch (e) {
      setAddErr(`${C.fail.add} ${reason(e)}`.trim());
    } finally {
      setAddBusy(false);
    }
  };

  const agent = prefs.view?.effective.agent;
  const start = async () => {
    if (!picked || m.ready < 3 || inFlight.current) return;
    if (!ask.trim()) {
      setAskErr(C.ask.empty);
      root.current?.querySelector<HTMLElement>(".v-wel__ask textarea")?.focus();
      return;
    }
    inFlight.current = true;
    setBusy(true);
    setFail("");
    setAskErr("");
    const eff = prefs.view?.effective;
    const input = toInput({ repo: picked.root, ask, base: "HEAD", rounds: String(eff?.rounds ?? 3), maxInFlight: String(eff?.workers ?? 4) });
    try {
      // The same call Start a run makes: one fresh action id per click, so a retry never starts a second run.
      const r = await api.actStartRun({ action_id: newActionId(), project_id: picked.id, expect_generation: null }, { ...input, ...(agent ? { provider: agent } : {}) }, picked.root);
      if (r.receipt?.state !== "done" || !r.result) throw r.error ?? { error: r.receipt?.reason ?? "failed", detail: r.receipt?.detail ?? undefined };
      onStarted();
      go("run", { slug: r.result.campaign, id: r.result.run_id, stream: r.result.stream, first: "1" }, picked.id);
    } catch (e) {
      setFail(`${C.fail.start} ${reason(e)}`.trim());
      setBusy(false);
      inFlight.current = false;
    }
  };

  const left = 3 - m.ready;
  const why = left > 0 ? C.finish(left) : "";
  return (
    <div className="v-screen v-wel" ref={root}>
      <div className="v-screen__body">
        <div className="v-wel__col">
          <Text as="h1" variant="headline-lg">{C.title}</Text>
          <Text variant="body" muted className="v-wel__lede">{C.lede}</Text>
          <div className="v-wel__list">
            <CheckList label={C.listLabel}>
              <CheckRow n={1} title={C.folder.title} kind={m.folder.kind} word={m.folder.word} summary={m.folder.summary} open={open === 1} onToggle={() => toggle(1)}>
                <FolderBody repos={repos} picked={picked} onUse={(p, o) => void use(p, o)} busy={addBusy} error={addErr} onLookAgain={scan} />
              </CheckRow>
              <CheckRow n={2} title={C.agent.title} kind={m.agent.kind} word={m.agent.word} summary={m.agent.summary} open={open === 2} onToggle={() => toggle(2)}>
                <AgentBody agents={agents} tools={tools} error={setup.error} retry={() => void recheck()} reload={reloadSetup}
                  onInstallTools={() => api.installDevTools().then(() => C.agent.toolsStarted)} />
              </CheckRow>
              <CheckRow n={3} title={C.engine.title} kind={m.engine.kind} word={m.engine.word} summary={m.engine.summary} open={open === 3} onToggle={() => toggle(3)}>
                <EngineBody engine={setup.list?.engine ?? null} />
              </CheckRow>
            </CheckList>
          </div>
          <section className="v-wel__ask" aria-labelledby="v-wel-ask">
            <h2 id="v-wel-ask" className="v-wel__askh"><span className="v-chk__n" aria-hidden="true">4</span>{C.ask.title}</h2>
            <Input type="textarea" rows={3} label={C.ask.label} value={ask} onChange={(v) => { setAsk(v); setAskErr(""); }} error={askErr || undefined}
              hint={`${C.ask.hint}${picked ? ` ${C.ask.runsIn(picked.name)}` : ""}`} />
            {fail && <div role="alert" className="v-launch__fail"><Text variant="body-sm">{fail}</Text></div>}
          </section>
        </div>
      </div>
      <ActionBar label={C.start}
        context={
          <>
            <span>{m.ready === 3 ? C.ready : C.progress(m.ready)}</span>
            <span className="v-wel__hint"><Kbd keys={["↑"]} decorative /><Kbd keys={["↓"]} decorative />{C.move} · <Kbd keys={["↵"]} decorative />{C.open}</span>
            {why && <span id="v-wel-why" role="status">{why}</span>}
          </>
        }
        actions={[{ id: "start", label: busy ? C.starting : C.start, keys: START_KEYS, primary: true, disabled: m.ready < 3, busy, run: () => void start() }]} />
    </div>
  );
}
