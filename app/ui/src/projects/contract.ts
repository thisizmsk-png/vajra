// The project shapes the shell reads. The contract itself is T1's `src/projectContract.ts` (the TS mirror of the core's
// `vajra.project/1` and `vajra.session/1`); these are the UI's views of it. Until the core serves `project_list`,
// `adapter.ts` derives them from today's campaign, crew and inbox commands; then its loaders call the core and the
// shell's components do not change.
import type { StatusKey } from "../../strings";
import type { Project as ContractProject, SessionStatus } from "../projectContract";

export type { SessionStatus };
export type ProjectId = ContractProject["id"];

/**
 * A contract project as the shell shows it. Until `project_list` lands the adapter has no `root_key` (the core's
 * dev/ino identity) and records `root` as the run recorded it, not canonicalised. `bucket: "unsorted"` is ADR-15
 * §1.5's Imported group: campaigns with no recorded folder, so `root` is null. The adapter's ids are `p_<8 hex>`
 * placeholders, not the core's ULID-style ids; they are never sent to the core.
 */
export type Project = Pick<ContractProject, "schema" | "id" | "name" | "rev"> & {
  root: string | null;
  created: string | null;
  archived: ContractProject["archived"];
  bucket?: "unsorted";
};

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
