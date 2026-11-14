// Derives the project view from today's commands (campaigns, resume targets, crew, run overview, evidence, inbox).
// Pure functions only, so the swap to the core's project contract is a change of inputs, tested by the same cases.
// runs.jsonl (behind resume_targets) is worker-writable and untrusted (ADR-15 §0): it is used here for grouping and
// display only, exactly as Home already uses it to offer Resume. It never authorises anything.
import { S } from "../../strings";
import type { StatusKey } from "../../strings";
import type { Campaign, EvidenceRow, InboxItem, ResumeTarget, RunOverview } from "../ipc";
import { PROJECT_SCHEMA, validId } from "../projectContract";
import type { Project as StoreProject } from "../projectContract";
import { parseWorkerProvider } from "../workerProvider";
import type { ActiveCrew, BoardCard, DayGroup, Lane, Project, ProjectId, ProjectSummary, SessionRow, SessionStatus, TabState } from "./contract";

export const UNSORTED_ID = "p_unsorted";

export const normRoot = (p: string) => {
  const t = p.trim();
  const s = t.replace(/\/+$/, "");
  return s === "" && t.startsWith("/") ? "/" : s;
};
const basename = (p: string) => p.split("/").filter(Boolean).pop() ?? p;

/** A stable id for a folder: FNV-1a of the recorded path, safe in a URL path segment. The core's ids (ULIDs) replace it. */
export function projectId(root: string): ProjectId {
  let h = 0x811c9dc5;
  for (const ch of normRoot(root)) {
    h ^= ch.codePointAt(0)!;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return `p_${h.toString(16).padStart(8, "0")}`;
}

/** The time a campaign is filed under: its last update, else its creation. */
const whenOf = (c: Campaign) => c.updated ?? c.created ?? "";
const firstSeen = (c: Campaign) => c.created ?? c.updated ?? "";

export type ProjectIndex = { projects: Project[]; byCampaign: Record<string, ProjectId> };

/**
 * One project per recorded run folder; campaigns with no folder go to the Unsorted bucket, never to a guessed project
 * (FG-01). Projects are ordered by their oldest campaign so a new run never moves a tab (⌘1..n stay put).
 */
export function groupProjects(campaigns: Campaign[] | undefined, targets: Record<string, ResumeTarget> | undefined, store: StoreProject[] = []): ProjectIndex {
  const roots = new Map<string, { root: string; first: string }>();
  const byCampaign: Record<string, ProjectId> = {};
  let unsorted = null as string | null;
  // The core's projects (#77) come first, in its order, with their T1 ids, even before they have a campaign. A folder
  // that is not registered keeps a placeholder until the owner adds it (ADR-15 D2: nothing is assigned for them).
  const registered: Project[] = [];
  const byRoot = new Map<string, ProjectId>();
  for (const p of store ?? []) {
    if (!p || p.archived || !validId("p", p.id) || typeof p.root !== "string" || !p.root.trim() || byRoot.has(normRoot(p.root))) continue;
    registered.push({ schema: PROJECT_SCHEMA, id: p.id, name: p.name || basename(p.root), root: normRoot(p.root), created: p.created ?? null, archived: null, rev: p.rev ?? 0, registered: true });
    byRoot.set(normRoot(p.root), p.id);
  }
  for (const c of campaigns ?? []) {
    const cwd = targets?.[c.slug]?.cwd;
    const root = typeof cwd === "string" && cwd.trim() ? normRoot(cwd) : null;
    if (!root) {
      byCampaign[c.slug] = UNSORTED_ID;
      unsorted = unsorted === null || firstSeen(c) < unsorted ? firstSeen(c) : unsorted;
      continue;
    }
    const known = byRoot.get(root);
    if (known) { byCampaign[c.slug] = known; continue; }
    const id = projectId(root);
    byCampaign[c.slug] = id;
    const seen = roots.get(id);
    const first = firstSeen(c) || "￿";
    if (!seen || first < seen.first) roots.set(id, { root, first });
  }
  const projects: Project[] = [...registered, ...[...roots.entries()]
    .sort(([, a], [, b]) => (a.first === b.first ? a.root.localeCompare(b.root) : a.first < b.first ? -1 : 1))
    .map(([id, r]): Project => ({ schema: PROJECT_SCHEMA, id, name: basename(r.root), root: r.root, created: r.first === "￿" ? null : r.first, archived: null, rev: 0 }))];
  if (unsorted !== null) {
    projects.push({ schema: PROJECT_SCHEMA, id: UNSORTED_ID, name: S.projects.unsorted, root: null, created: unsorted || null, archived: null, rev: 0, bucket: "unsorted" });
  }
  return { projects, byCampaign };
}

const SESSION: Record<string, SessionStatus> = { active: "running", running: "running", done: "done", delivered: "done", stopped: "stopped", abandoned: "stopped", failed: "failed" };
export const sessionStatus = (c: Campaign): SessionStatus => SESSION[c.status ?? ""] ?? "unknown";
const DOT: Record<SessionStatus, StatusKey> = { running: "in_progress", done: "done", stopped: "blocked", failed: "failed", unknown: "unknown" };

type Owned = { campaign?: string | null; project_id?: string };

/**
 * The tab a campaign item is filed under: the project the core names (its store id), else the project of the item's
 * campaign. `p_unsorted` from the core means "no registered project owns the folder", and a campaign whose folder is
 * known still has its own tab, so that case falls back too. Nothing is dropped: no tab at all means Unsorted.
 */
export function itemProject(index: ProjectIndex, item: Owned): ProjectId | undefined {
  const core = item.project_id;
  if (typeof core === "string" && core !== UNSORTED_ID && index.projects.some((p) => p.id === core)) return core;
  return item.campaign ? index.byCampaign[item.campaign] : undefined;
}

/** The project id an action on this item names: exactly the core's (it checks the item against it), else its campaign's tab. */
export function actionProject(index: ProjectIndex, item: Owned): ProjectId | undefined {
  return typeof item.project_id === "string" && (item.project_id === UNSORTED_ID || validId("p", item.project_id)) ? item.project_id : item.campaign ? index.byCampaign[item.campaign] : undefined;
}

/** The tab of each project: needs-you count, the urgent state, and worker pips for the project that owns the live run. */
export function summarize(index: ProjectIndex, campaigns: Campaign[], inbox: InboxItem[], crew: ActiveCrew | null): Map<ProjectId, ProjectSummary> {
  const out = new Map<ProjectId, ProjectSummary>();
  for (const p of index.projects) out.set(p.id, { needsYou: 0, state: null, pips: [], live: false });
  // A run proposal names its project, not a campaign (FG-02b): by the core's id when it is a registered project, else
  // by its full root.
  const byRoot = new Map<string, ProjectId>();
  for (const p of index.projects) if (p.root) byRoot.set(normRoot(p.root), p.id);
  for (const item of inbox) {
    const pid = item.campaign ? itemProject(index, item)
      : typeof item.project_id === "string" && out.has(item.project_id) ? item.project_id
      : typeof item.project === "string" ? byRoot.get(normRoot(item.project)) : undefined;
    const s = pid ? out.get(pid) : undefined;
    if (s) s.needsYou += 1;
  }
  const newest = new Map<ProjectId, Campaign>();
  for (const c of campaigns) {
    const id = index.byCampaign[c.slug];
    const n = newest.get(id);
    if (id && (!n || whenOf(c) > whenOf(n))) newest.set(id, c);
  }
  for (const [id, s] of out) {
    const st = newest.get(id) ? sessionStatus(newest.get(id)!) : "unknown";
    const state: TabState = s.needsYou > 0 ? "needs_you" : st === "stopped" ? "stopped" : st === "failed" ? "failed" : null;
    s.state = state;
  }
  if (crew) {
    const s = out.get(index.byCampaign[crew.campaign]);
    if (s) { s.pips = crew.workers.map((w) => w.status); s.live = true; }
  }
  return out;
}

const DAY_RE = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?/;
const dayKey = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const ORDER: SessionStatus[] = ["running", "done", "stopped", "failed", "unknown"];

/** A project's runs by day, newest first, each day with a status summary ("2 done, 1 stopped"). Unreadable times go to Earlier. */
export function sessionsByDay(campaigns: Campaign[], now: Date = new Date()): DayGroup[] {
  const today = dayKey(now);
  const y = new Date(now);
  y.setDate(y.getDate() - 1);
  const yesterday = dayKey(y);
  const groups = new Map<string, DayGroup>();
  const rows = campaigns.map((c) => {
    const m = DAY_RE.exec(whenOf(c));
    const status = sessionStatus(c);
    const row: SessionRow = { slug: c.slug, title: c.goal || c.slug, status, dot: DOT[status], time: m?.[4] ? `${m[4]}:${m[5]}` : "", sortKey: m ? whenOf(c) : "" };
    return { row, day: m ? `${m[1]}-${m[2]}-${m[3]}` : "" };
  }).sort((a, b) => (a.row.sortKey === b.row.sortKey ? 0 : a.row.sortKey > b.row.sortKey ? -1 : 1));
  for (const { row, day } of rows) {
    if (!groups.has(day)) {
      const [yy, mm, dd] = day.split("-").map(Number);
      const label = !day ? S.projects.earlier : day === today ? S.projects.today : day === yesterday ? S.projects.yesterday
        : new Date(yy, mm - 1, dd).toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" });
      groups.set(day, { key: day || "earlier", label, summary: "", items: [] });
    }
    groups.get(day)!.items.push(row);
  }
  const out = [...groups.values()].sort((a, b) => (a.key === "earlier" ? 1 : b.key === "earlier" ? -1 : a.key > b.key ? -1 : 1));
  for (const g of out) {
    const n = new Map<SessionStatus, number>();
    for (const r of g.items) n.set(r.status, (n.get(r.status) ?? 0) + 1);
    g.summary = ORDER.filter((s) => n.has(s)).map((s) => S.projects.dayCount(n.get(s)!, s)).join(", ");
  }
  return out;
}

const TASK: Record<string, StatusKey> = { pending: "pending", ready: "ready", claimed: "claimed", in_progress: "in_progress", done: "done", blocked: "blocked", question: "question", failed: "failed" };

/**
 * The board of one campaign (ADR-15 §4.2, D6 A, D7 read-only): Backlog, Running, Review, Done, plus a needs-you badge.
 * A card reaches Done only when every one of its criteria is verified passing; a worker saying "done" is not enough.
 */
export function boardCards(overview: RunOverview, evidence: EvidenceRow[], inbox: InboxItem[]): BoardCard[] {
  const cards: BoardCard[] = overview.graph.tasks.map((t) => {
    const rows = evidence.filter((r) => r.task === t.id || r.task === t.local);
    const pass = rows.filter((r) => r.status === "pass").length;
    const status = TASK[t.status] ?? "unknown";
    let lane: Lane;
    if (status === "claimed" || status === "in_progress" || status === "question") lane = "running";
    else if (status === "done") lane = rows.length > 0 && pass === rows.length ? "done" : "review";
    else if (status === "failed") lane = "review";
    else lane = "backlog";
    return {
      key: `task:${t.id}`, kind: "task", local: t.local, title: t.title ?? t.local, owner: t.owner, provider: parseWorkerProvider(t.provider), status, lane,
      needsYou: status === "question" || status === "blocked",
      criteria: rows.length ? { pass, total: rows.length } : null,
    };
  });
  for (const item of inbox) {
    if (item.campaign !== overview.slug || (item.kind !== "plan" && item.kind !== "question")) continue;
    cards.push({ key: `inbox:${item.id}`, kind: item.kind, local: item.id, title: item.title, owner: null, status: "needs_approval", lane: "review", needsYou: true, criteria: null });
  }
  return cards;
}
