// One map from what the core says about a run or a task to the status vocabulary (DESIGN.md "Status vocabulary"), so a
// state looks and reads the same in the project column, the Run view, History and the Board: StatusDot's shape, colour
// and word. Pills (Badge) stay for counts, integrity and "Needs you", never for a run's or a task's state.
import type { StatusKey } from "../strings";

/** A task in the run graph: closed and done are both Done. Anything new reads as its own word on the "Not run" ring. */
const TASK: Record<string, StatusKey> = {
  pending: "pending", ready: "ready", claimed: "claimed", in_progress: "in_progress", question: "question",
  blocked: "blocked", closed: "done", done: "done", failed: "failed",
};
export const taskStatusKey = (s: string): StatusKey | null => TASK[s] ?? null;

/** A run (campaign) in History. Stopped is the square, as in the project column's sessions list. */
const RUN: Record<string, StatusKey> = { active: "in_progress", done: "done", stopped: "blocked", failed: "failed", abandoned: "unknown" };
export const runStatusKey = (s: string): StatusKey => RUN[s] ?? "unknown";

/** The run's own progress (run_overview's timeline). */
const TIMELINE: Record<string, StatusKey> = { active: "in_progress", done: "done", need_input: "question", rejected: "failed" };
export const timelineStatusKey = (s: string): StatusKey => TIMELINE[s] ?? "unknown";
