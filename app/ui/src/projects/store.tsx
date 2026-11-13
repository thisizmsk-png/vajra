// The shell's one copy of the project view. It reads today's commands through the adapter; when the core serves
// `project_state` (ADR-15 §3.3), `load` calls that instead and the components keep reading `useProjects()`.
// Idle cost: one read at start and one per `changed` burst. It polls only while a run is live and the window is visible
// (the crew and the inbox change without events), the same 2 s the Crew screen already uses.
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { S } from "../../strings";
import type { StatusKey } from "../../strings";
import { api } from "../ipc";
import { forgetRead, ownedRead } from "../reads";
import type { Campaign, CrewNodeDto, InboxItem } from "../ipc";
import { problemDetail } from "../screens/states";
import { parseWorkerProvider } from "../workerProvider";
import { groupProjects, summarize } from "./adapter";
import { onChangedPaced, usePoll } from "./poll";
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
  /** The inbox read failed this time: `inbox` is empty because it is unknown, not because nothing waits. */
  inboxFailed: boolean;
  /** The core's full pending total; `inbox` is cut at 200, so the badge uses this. */
  inboxCount: number;
  /** The core's total without `run_proposal` items: what the Inbox badge counts. `inboxCount` stays the "needs you" total.
   * null when the core did not send a usable one (an older core): the badge then counts the list it has. */
  inboxCampaignCount: number | null;
  crew: ActiveCrew | null;
  /** How many runs the core says are live (today at most one; ADR-15 D5 raises it). */
  runs: number;
  /** The run ids the core says are live: a screen polls its run only while it is one of these. */
  liveRuns: string[];
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
      out.push({ id: n.id.slice("agent:".length), name: n.role ?? n.title, status, statusLabel: S.status[status], task, provider: parseWorkerProvider(n.provider) });
    }
    for (const c of n.children) walk(c, n.kind === "task" ? n.title : task);
  };
  walk(root, "");
  return out;
}

const EMPTY: ProjectIndex = { projects: [], byCampaign: {} };
const Ctx = createContext<ProjectsView>({
  state: "loading", error: "", ...EMPTY, runIds: {}, campaigns: [], inbox: [], inboxCount: 0, inboxCampaignCount: null, inboxFailed: false, crew: null, runs: 0, liveRuns: [], summaries: new Map(), reload: () => {},
});

export function ProjectsProvider({ children }: { children: ReactNode }) {
  const [data, setData] = useState<Omit<ProjectsView, "reload">>({ state: "loading", error: "", ...EMPTY, runIds: {}, campaigns: [], inbox: [], inboxCount: 0, inboxCampaignCount: null, inboxFailed: false, crew: null, runs: 0, liveRuns: [], summaries: new Map() });
  const seq = useRef(0);

  const load = useCallback(() => {
    const mine = ++seq.current;
    void (async () => {
      // Campaigns and their folders decide the projects; without either, saying "no projects" would be wrong.
      const [camps, targets, active, inbox, store] = await Promise.allSettled([
        ownedRead("list_campaigns", () => api.listCampaigns()), ownedRead("resume_targets", () => api.resumeTargets()),
        ownedRead("active_runs", () => api.activeRuns()), ownedRead("inbox", () => api.inbox()), ownedRead("project_list", () => api.projectList()),
      ]);
      if (mine !== seq.current) return;
      if (camps.status === "rejected" || targets.status === "rejected") {
        const e = camps.status === "rejected" ? camps.reason : (targets as PromiseRejectedResult).reason;
        // A successful inbox read is still the newest answer: the Board shares it, so it must not keep an older one.
        const fresh = inbox.status === "fulfilled" && Array.isArray(inbox.value?.items)
          ? { inbox: inbox.value.items, inboxFailed: false, inboxCount: Number.isSafeInteger(inbox.value.count) && inbox.value.count >= inbox.value.items.length ? inbox.value.count : inbox.value.items.length }
          : {};
        setData((d) => ({ ...d, ...fresh, state: "error", error: problemDetail(e) }));
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
      const liveRuns = active.status === "fulfilled" && Array.isArray(active.value?.runs)
        ? (active.value.runs as { run_id?: unknown }[]).map((r) => r.run_id).filter((r): r is string => typeof r === "string")
        : [];
      // The core's campaign-only total when it is sane (a whole number, at least what the list shows, at most the total);
      // else null: an older core sent none, and the badge must not mistake the cut list for the total.
      const listed = items.filter((i) => i.kind !== "run_proposal").length;
      const cc = inbox.status === "fulfilled" ? inbox.value?.campaign_count : undefined;
      const campaignCount = typeof cc === "number" && Number.isSafeInteger(cc) && cc >= listed && cc <= (inbox.status === "fulfilled" ? inbox.value.count : listed) ? cc : null;
      setData({ state: "ok", error: "", ...index, runIds, campaigns, inbox: items, inboxCount: inbox.status === "fulfilled" && Number.isSafeInteger(inbox.value?.count) && inbox.value.count >= items.length ? inbox.value.count : items.length, inboxCampaignCount: campaignCount, inboxFailed: inbox.status === "rejected", crew, runs, liveRuns, summaries: summarize(index, campaigns, items, crew) });
    })();
  }, []);

  useEffect(load, [load]);
  const live = data.crew !== null;
  // Paced, and held back while the window is hidden (one read on showing).
  useEffect(() => onChangedPaced(EVENT_MIN_GAP_MS, load, () => {
    // The core says something moved: a read already in flight started before that and must not be joined.
    forgetRead();
    return true;
  }), [load]);
  usePoll(load, POLL_MS, live);

  const view = useMemo(() => ({ ...data, reload: load }), [data, load]);
  return <Ctx.Provider value={view}>{children}</Ctx.Provider>;
}

export const useProjects = () => useContext(Ctx);
