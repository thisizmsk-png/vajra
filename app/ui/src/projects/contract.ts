// The project shapes the shell reads, mirrored from ADR-15 §1.2 (`vajra.project/1`, `vajra.session/1`) and §4.2 (board).
// The core does not serve them yet: `adapter.ts` derives them from today's campaign, crew and inbox commands. When the
// core's contract lands (T1 PR 1a, then T2's `project_state`), its types replace these and the adapter's loaders call
// the core instead; the shell's components do not change. Nothing here is a core contract: it is the UI's reading of one.
import type { StatusKey } from "../../strings";

export type ProjectId = string;

/** ADR-15 §1.2. `bucket: "unsorted"` is the Imported (unsorted) group of §1.5: campaigns with no recorded folder. */
export type Project = {
  schema: "vajra.project/1";
  id: ProjectId;
  name: string;
  /** The folder runs were started in, as recorded (not canonicalised: the core does that, by realpath and dev/ino). */
  root: string | null;
  created: string | null;
  archived: null;
  rev: number;
  bucket?: "unsorted";
};

/** A run's state as a session (`vajra.session/1` `status`). */
export type SessionStatus = "running" | "stopped" | "done" | "failed" | "unknown";

/** One row of a project's history: a campaign run, until the core lists sessions itself. */
export type SessionRow = { slug: string; title: string; status: SessionStatus; dot: StatusKey; time: string; sortKey: string };
export type DayGroup = { key: string; label: string; summary: string; items: SessionRow[] };

/** A worker of the project's live run, from the crew tree. */
export type WorkerRow = { id: string; name: string; status: StatusKey; statusLabel: string; task: string };
/** The live run and its workers. Today at most one run is active in the whole app (`no_active_run`). */
export type ActiveCrew = { campaign: string; runId: string; workers: WorkerRow[] };

/** What a project tab shows: the urgent state first (FG-03 "urgent items stay visible"). Never "paused": nothing reports it yet. */
export type TabState = "needs_you" | "stopped" | "failed" | null;
export type ProjectSummary = { needsYou: number; state: TabState; pips: StatusKey[]; live: boolean };

/** ADR-15 §4.2 lanes (D6 option A). Needs-you is a badge, not a lane. */
export type Lane = "backlog" | "running" | "review" | "done";
export type BoardCard = {
  key: string;
  kind: "task" | "plan" | "question";
  /** The task's run-local id (`t3`), or the inbox item id for a plan or question. */
  local: string;
  title: string;
  owner: string | null;
  status: StatusKey;
  lane: Lane;
  needsYou: boolean;
  criteria: { pass: number; total: number } | null;
};
