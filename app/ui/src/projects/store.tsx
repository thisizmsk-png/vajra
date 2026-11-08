// The shell's one copy of the project view. It reads today's commands through the adapter; when the core serves
// `project_state` (ADR-15 §3.3), `load` calls that instead and the components keep reading `useProjects()`.
// Idle cost: one read at start and one per `changed` burst. It polls only while a run is live and the window is visible
// (the crew and the inbox change without events), the same 2 s the Crew screen already uses.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { S } from "../../strings";
import type { StatusKey } from "../../strings";
import { api, onChanged } from "../ipc";
import type { Campaign, CrewNodeDto, InboxItem } from "../ipc";
import { problemDetail } from "../screens/states";
import { groupProjects, summarize } from "./adapter";
import type { ProjectIndex } from "./adapter";
import type { ActiveCrew, Project, ProjectId, ProjectSummary, WorkerRow } from "./contract";

const POLL_MS = 2000;
const EVENT_MIN_GAP_MS = 500;

export type ProjectsView = {
  state: "loading" | "ok" | "error";
  error: string;
  projects: Project[];
  byCampaign: Record<string, ProjectId>;
  /** The run id of each campaign this app started (`resume_targets`), for the Director-General's commands. */
  runIds: Record<string, string>;
  campaigns: Campaign[];
  inbox: InboxItem[];
  crew: ActiveCrew | null;
  /** How many runs the core says are live (today at most one; ADR-15 D5 raises it). */
  runs: number;
  summaries: Map<ProjectId, ProjectSummary>;
  reload: () => void;
};

const STATUSES = new Set<string>(["pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed"]);
const statusOf = (s: string): StatusKey => (STATUSES.has(s) ? (s as StatusKey) : "unknown");

/** The workers of a crew tree, each with the title of the task it is under. */
export function workersOf(root: CrewNodeDto): WorkerRow[] {
  const out: WorkerRow[] = [];
  const walk = (n: CrewNodeDto, task: string) => {
    if (n.id.startsWith("agent:")) {
      const status = statusOf(n.status);
      out.push({ id: n.id.slice("agent:".length), name: n.role ?? n.title, status, statusLabel: S.status[status], task });
    }
    for (const c of n.children) walk(c, n.kind === "task" ? n.title : task);
  };
  walk(root, "");
  return out;
}

const EMPTY: ProjectIndex = { projects: [], byCampaign: {} };
const Ctx = createContext<ProjectsView>({
  state: "loading", error: "", ...EMPTY, runIds: {}, campaigns: [], inbox: [], crew: null, runs: 0, summaries: new Map(), reload: () => {},
});

export function ProjectsProvider({ children }: { children: ReactNode }) {
  const [data, setData] = useState<Omit<ProjectsView, "reload">>({ state: "loading", error: "", ...EMPTY, runIds: {}, campaigns: [], inbox: [], crew: null, runs: 0, summaries: new Map() });
  const seq = useRef(0);
  const last = useRef(0);

  const load = useCallback(() => {
    const mine = ++seq.current;
    last.current = Date.now();
    void (async () => {
      // Campaigns and their folders decide the projects; without either, saying "no projects" would be wrong.
      const [camps, targets, active, inbox, store] = await Promise.allSettled([api.listCampaigns(), api.resumeTargets(), api.activeRuns(), api.inbox(), api.projectList()]);
      if (mine !== seq.current) return;
      if (camps.status === "rejected" || targets.status === "rejected") {
        const e = camps.status === "rejected" ? camps.reason : (targets as PromiseRejectedResult).reason;
        setData((d) => ({ ...d, state: "error", error: problemDetail(e) }));
        return;
      }
      const campaigns = Array.isArray(camps.value?.campaigns) ? camps.value.campaigns : [];
      const tmap = targets.value?.targets ?? {};
      // The core's project store (#77) gives registered projects their T1 ids; an older core, or a failed read, leaves
      // every project as a folder placeholder, exactly as before.
      const registered = store.status === "fulfilled" && Array.isArray(store.value?.projects) ? store.value.projects : [];
      const index = groupProjects(campaigns, tmap, registered);
      const runIds: Record<string, string> = {};
      for (const [slug, t] of Object.entries(tmap)) if (t && typeof t.run_id === "string") runIds[slug] = t.run_id;
      // The inbox and the live run add urgency; if they fail the projects still show, without those marks.
      const items = inbox.status === "fulfilled" && Array.isArray(inbox.value?.items) ? inbox.value.items : [];
      const run = active.status === "fulfilled" && Array.isArray(active.value?.runs)
        ? (active.value.runs as { run_id?: unknown; campaign?: unknown }[]).find((r) => typeof r.run_id === "string" && typeof r.campaign === "string")
        : undefined;
      let crew: ActiveCrew | null = null;
      if (run) {
        try {
          const root = await api.crew(run.campaign as string, run.run_id as string);
          crew = { campaign: run.campaign as string, runId: run.run_id as string, workers: root && Array.isArray(root.children) ? workersOf(root) : [] };
        } catch {
          crew = { campaign: run.campaign as string, runId: run.run_id as string, workers: [] };
        }
      }
      // A newer load started while the crew was asked for: whatever this one learned (answer or failure) is stale.
      if (mine !== seq.current) return;
      const runs = active.status === "fulfilled" && Array.isArray(active.value?.runs) ? active.value.runs.length : 0;
      setData({ state: "ok", error: "", ...index, runIds, campaigns, inbox: items, crew, runs, summaries: summarize(index, campaigns, items, crew) });
    })();
  }, []);

  useEffect(load, [load]);
  const live = data.crew !== null;
  useEffect(() => {
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged(() => {
      const wait = EVENT_MIN_GAP_MS - (Date.now() - last.current);
      if (wait <= 0) load();
      else if (later === undefined) later = setTimeout(() => { later = undefined; load(); }, wait);
    });
    const poll = live ? setInterval(() => { if (!document.hidden) load(); }, POLL_MS) : undefined;
    return () => { off(); clearTimeout(later); clearInterval(poll); };
  }, [load, live]);

  const view = useMemo(() => ({ ...data, reload: load }), [data, load]);
  return <Ctx.Provider value={view}>{children}</Ctx.Provider>;
}

export const useProjects = () => useContext(Ctx);
