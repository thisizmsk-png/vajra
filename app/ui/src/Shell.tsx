import { useEffect, useMemo, useRef, useState } from "react";
import { Icon, Kbd, ariaShortcut } from "../components/ui/base";
import { ErrorBoundary } from "./ErrorBoundary";
import { PaletteHost, usePaletteOpener } from "./palette";
import { toHash, useRoute } from "./router";
import type { Screen } from "./router";
import { REGISTRY } from "./screens/registry";
import { ProjectsProvider, useProjects } from "./projects/store";
import { CurrentProject } from "./projects/current";
import type { Current } from "./projects/current";
import { ProjectTabs, stateText } from "./shell/ProjectTabs";
import { ProjectColumn } from "./shell/ProjectColumn";
import { ViewTabs } from "./shell/ViewTabs";
import type { WorkerTab } from "./shell/ViewTabs";
import { Inspector } from "./shell/Inspector";
import { StatusBar } from "./shell/StatusBar";
import { lastProject, rememberProject, useCompact, useInspector } from "./shell/layout";
import { S } from "../strings";
import { PreferencesProvider, usePreferences } from "./preferences";
import { DEFAULT_PREFERENCES, matchesShortcut, shortcutKeys } from "./preferenceContract";
import "./shell.css";
import "./shell/frame.css";

/** In the macOS app the title bar is an overlay (tauri.conf.json): the traffic lights sit at the left of the title row. */
const OVERLAY = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && /Mac/.test(navigator.userAgent);
/** Views that have something to say in the inspector. Review has its own panes; Settings and the legacy views have none. */
const INSPECTED: ReadonlySet<Screen> = new Set(["orch", "board", "worker"]);
/** Views that make sense in another project; anything tied to one campaign or worker becomes the Director-General. */
const PORTABLE: ReadonlySet<Screen> = new Set(["orch", "board", "history", "settings"]);
const MAX_WORKER_TABS = 4;

/** Shell B: project tabs in the title bar, the project column, the main area with its view tabs, the inspector, the status bar. */
export function Shell() {
  return (
    <ProjectsProvider>
      <Frame />
    </ProjectsProvider>
  );
}

function Frame() {
  const P = useProjects();
  const [route, go] = useRoute();
  const [compact, setCompact] = useCompact();
  const [inspector, setInspector] = useInspector();
  const main = useRef<HTMLElement>(null);

  // Which project you are in: the one that owns the campaign in the link, the one in the hash, the last one you used,
  // the one with the live run, else the first. `new` is the launcher for a folder that is not a project yet.
  const known = (id: string | null | undefined): id is string => !!id && P.projects.some((p) => p.id === id);
  const slug = route.params.slug;
  const live = P.crew ? P.byCampaign[P.crew.campaign] : undefined;
  const remembered = lastProject();
  const pid: string | null = route.params.new ? null
    : known(slug ? P.byCampaign[slug] : undefined) ? P.byCampaign[slug!]
    : known(route.project) ? route.project
    : known(remembered) ? remembered
    : known(live) ? live
    : P.projects[0]?.id ?? null;
  const project = P.projects.find((p) => p.id === pid) ?? null;
  useEffect(() => { if (pid) rememberProject(pid); }, [pid]);

  const current: Current = useMemo(() => {
    const campaigns = project ? P.campaigns.filter((c) => P.byCampaign[c.slug] === project.id)
      .sort((a, b) => ((b.updated ?? b.created ?? "") > (a.updated ?? a.created ?? "") ? 1 : -1)) : [];
    const mine = P.crew && project && P.byCampaign[P.crew.campaign] === project.id ? P.crew : null;
    return { project, campaigns, focus: mine?.campaign ?? campaigns[0]?.slug ?? null, runId: mine?.runId ?? null };
  }, [project, P.campaigns, P.byCampaign, P.crew]);
  const crew = current.runId ? P.crew : null;
  const focusCampaign = current.campaigns.find((c) => c.slug === current.focus) ?? null;

  // Worker tabs: one per worker you opened in this project, newest last, at most four.
  const [workerTabs, setWorkerTabs] = useState<WorkerTab[]>([]);
  useEffect(() => { setWorkerTabs([]); }, [pid]);
  const { worker: wParam = "", id: wRun = "", slug: wSlug = "" } = route.params;
  useEffect(() => {
    if (route.screen !== "worker" || !wParam || !wRun) return;
    const name = P.crew?.workers.find((w) => w.id === wParam)?.name ?? wParam;
    setWorkerTabs((ts) => (ts.some((t) => t.worker === wParam && t.id === wRun) ? ts
      : [...ts, { slug: wSlug, id: wRun, worker: wParam, name }].slice(-MAX_WORKER_TABS)));
  }, [route.screen, wParam, wRun, wSlug, P.crew]);
  const closeTab = (t: WorkerTab) => {
    setWorkerTabs((ts) => ts.filter((x) => x !== t));
    if (route.screen === "worker" && route.params.worker === t.worker && route.params.id === t.id) go("orch", {}, pid);
  };

  // The window title names the view (WCAG 2.4.2).
  useEffect(() => {
    const view = route.screen === "worker" ? workerTabs.find((t) => t.worker === wParam)?.name ?? S.views.worker : S.views[route.screen];
    document.title = S.shell.title(project ? `${view}, ${project.name}` : view);
  }, [route.screen, project, workerTabs, wParam]);

  const goProject = (id: string) => go(PORTABLE.has(route.screen) ? route.screen : "orch", route.screen === "settings" ? route.params : {}, id);
  const openSettings = (params: Record<string, string> = {}) => go("settings", params, pid);
  const showColumn = !!project && !compact;
  const inspectable = !!project && INSPECTED.has(route.screen);
  const showInspector = inspectable && inspector;
  const reviewParams: Record<string, string> = current.focus ? { slug: current.focus } : {};

  // The window shortcuts read this render's projects and state through a ref, and the listener is attached once on
  // mount. Re-attaching it in an effect on every render left a gap after each render where a key (⌘1..9, ⌘B) was
  // handled with the previous render's state, or not at all (the flaky CI run on #86).
  const onKeyRef = useRef<(e: KeyboardEvent) => void>(() => {});
  onKeyRef.current = (e: KeyboardEvent) => {
    if (!e.metaKey || e.altKey || e.ctrlKey || e.defaultPrevented || e.isComposing) return;
    const k = e.key.toLowerCase();
    const n = Number(e.key);
    if (!e.shiftKey && Number.isInteger(n) && n >= 1 && n <= 9) {
      const p = P.projects[n - 1];
      if (p) { e.preventDefault(); goProject(p.id); }
    } else if (!e.shiftKey && k === "b" && project) { e.preventDefault(); setCompact((c) => !c); }
    else if (!e.shiftKey && k === "i" && inspectable) { e.preventDefault(); setInspector((x) => !x); }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => onKeyRef.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ⌘K: every project with its key, the views, the panels, Settings. The place you are on is not a place to go to.
  const commands = [
    ...P.projects.flatMap((p, i) => (p.id === pid ? [] : [{ id: `p-${p.id}`, label: S.frame.goProject(p.name), hint: stateText(P.summaries.get(p.id)) || undefined, keys: i < 9 ? ["⌘", String(i + 1)] : undefined, run: () => goProject(p.id) }])),
    ...(project ? (["orch", "board", "history"] as const).filter((v) => v !== route.screen).map((v) => ({ id: `v-${v}`, label: S.frame.show(S.views[v]), run: () => go(v, {}, pid) })) : []),
    ...(project && current.focus && route.screen !== "review" ? [{ id: "v-review", label: S.frame.show(S.views.review), run: () => go("review", reviewParams, pid) }] : []),
    ...(project ? [{ id: "column", label: compact ? S.frame.showColumn : S.frame.hideColumn, keys: ["⌘", "B"], run: () => setCompact((c) => !c) }] : []),
    ...(inspectable ? [{ id: "inspector", label: inspector ? S.frame.hideInspector : S.frame.showInspector, keys: ["⌘", "I"], run: () => setInspector((x) => !x) }] : []),
    ...(route.screen === "settings" ? [] : [{ id: "settings", label: S.frame.openSettings, keys: ["⌘", ","], run: () => openSettings() }]),
    { id: "add", label: S.frame.addProject, run: () => go("orch", { new: "1" }, null) },
  ];

  const skip = () => {
    const m = main.current;
    if (!m) return;
    const first = m.querySelector<HTMLElement>('input, textarea, select, button, a[href], [tabindex="0"]');
    (first ?? m).focus();
  };
  const Screen = REGISTRY[route.screen];
  const workerRow = route.screen === "worker" ? P.crew?.workers.find((w) => w.id === wParam) ?? null : null;

  return (
    <PreferencesProvider projectId={project && !project.bucket ? pid : null}>
      <PreferenceKeys openSettings={openSettings} />
    <PaletteHost commands={commands}>
      <CurrentProject.Provider value={current}>
        <div className={["v-app", OVERLAY && "v-app--overlay", showColumn ? "" : "v-app--nocol"].filter(Boolean).join(" ")}>
          <a className="v-skip" href="#v-main" onClick={(e) => { e.preventDefault(); skip(); }}>{S.shell.skip}</a>
          <header className="v-tb" data-tauri-drag-region>
            {P.projects.length > 0 ? (
              <ProjectTabs projects={P.projects} summaries={P.summaries} current={pid} view={route.screen} onAdd={() => go("orch", { new: "1" }, null)} />
            ) : (
              <>
                <span className="v-tb__empty">{P.state === "error" ? S.frame.loadFailed : P.state === "ok" ? S.frame.noProjects : ""}</span>
                {P.state === "error" && <button type="button" className="v-sb-btn" onClick={P.reload}>{S.states.retry}</button>}
                <button type="button" className="v-tb__add" onClick={() => go("orch", { new: "1" }, null)}><Icon name="plus" size={12} />{S.frame.addProject}</button>
              </>
            )}
            <Search />
          </header>
          {showColumn && project && (
            <ProjectColumn project={project} campaigns={current.campaigns} crew={crew} route={route}
              onOrch={() => go("orch", {}, pid)}
              onWorker={(w) => crew && go("worker", { slug: crew.campaign, id: crew.runId, worker: w.id }, pid)}
              onSession={(s) => go("review", { slug: s }, pid)} />
          )}
          <div className="v-center">
            {project && route.screen !== "settings" && (
              <ViewTabs route={route} project={pid} review={reviewParams} workers={workerTabs} onClose={closeTab}
                inspector={inspectable ? inspector : null} onInspector={() => setInspector((x) => !x)} />
            )}
            <div className="v-work">
              <main className="v-shell__main" id="v-main" tabIndex={-1} ref={main}>
                <ErrorBoundary resetKey={`${pid}|${toHash(route.screen, route.params)}`}>
                  <Screen key={route.screen === "orch" ? pid ?? "new" : undefined} />
                </ErrorBoundary>
              </main>
              {showInspector && project && <Inspector project={project} focus={focusCampaign} crew={crew} worker={workerRow} />}
            </div>
            {/* The terminal dock's place (FG-05/FG-16, the terminal track). Nothing mounts until its PR lands; then this div
                becomes `<TerminalDock defaultPlace={project?.root ? { cwd: project.root, label: project.name } : undefined} />`
                (src/terminal/TerminalDock.tsx, `Place = { cwd, label }`); the Unsorted bucket has no folder, so no default. */}
            <div className="v-dock" data-slot="terminal" hidden />
          </div>
          <StatusBar runs={P.runs} compact={project ? compact : null} onColumn={() => setCompact((c) => !c)}
            onPhone={() => openSettings({ section: "mobile" })} onSettings={() => openSettings()} />
        </div>
      </CurrentProject.Provider>
    </PaletteHost>
    </PreferencesProvider>
  );
}

/** The palette for the mouse, in the title bar. */
function Search() {
  const open = usePaletteOpener();
  const prefs = usePreferences();
  const keys = shortcutKeys(prefs.view?.effective.shortcuts.palette ?? DEFAULT_PREFERENCES.shortcuts.palette);
  return (
    <button type="button" className="v-tb__search" onClick={open} aria-keyshortcuts={ariaShortcut(keys)}>
      <Icon name="search" size={12} />
      <span>{S.frame.search}</span>
      <Kbd keys={keys} decorative />
    </button>
  );
}

function PreferenceKeys({ openSettings }: { openSettings: () => void }) {
  const prefs = usePreferences();
  const binding = prefs.view?.effective.shortcuts.settings ?? DEFAULT_PREFERENCES.shortcuts.settings;
  const latest = useRef(openSettings); latest.current = openSettings;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (!e.repeat && matchesShortcut(e, binding)) { e.preventDefault(); latest.current(); } };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [binding]);
  return null;
}
