// What the Workers screen says about a run as it happens: which worker is active, whether the run has stalled, and the
// worker's stream as messages and tool steps. Pure functions, so the rules are tested without a screen.
import { sanitizeLine } from "../../../components/ui/desktop/ansi";
import type { CrewNodeDto } from "../../ipc";

/** Nothing new for this long while a run is live counts as stalled (an agent that works writes far more often). */
export const STALL_MS = 30 * 60 * 1000;
/** The view watches a live run this long before calling it silent: the first reads of a stream need a moment. */
export const WATCH_MS = 60 * 1000;

const WORKING = new Set(["in_progress", "claimed"]);
export const isWorking = (status: string) => WORKING.has(status);

/** A worker the core runs itself, or a run's subagent: the nodes that have a pane. */
export const isWorkerId = (id: string) => id.startsWith("agent:") || id.startsWith("core:");

/** Every worker under `n`, in tree order. */
export function workersIn(n: CrewNodeDto, out: CrewNodeDto[] = []): CrewNodeDto[] {
  if (isWorkerId(n.id)) out.push(n);
  n.children.forEach((c) => workersIn(c, out));
  return out;
}

/** How much a working worker shows what is happening: one reporting activity now, then one with a stream to watch (a
 * run's subagent; the core's own workers write none). */
const rank = (w: CrewNodeDto) => ((w.overlays as unknown[]).includes("activity") ? 2 : 0) + (w.id.startsWith("agent:") ? 1 : 0);

/** The worker doing something now: of those working, the best ranked, the last in tree order on a tie (the newest round). */
export function activeWorker(n: CrewNodeDto): CrewNodeDto | null {
  let best: CrewNodeDto | null = null;
  for (const w of workersIn(n)) if (isWorking(w.status) && (!best || rank(w) >= rank(best))) best = w;
  return best;
}

/** The worker whose stream best tells what happened: the active one, else the last one that exists. */
export function focusWorker(n: CrewNodeDto): CrewNodeDto | null {
  return activeWorker(n) ?? workersIn(n).pop() ?? null;
}

/** What the screen selects when it opens: the active worker, else the newest worker, else the run, else the root. */
export function initialPick(root: CrewNodeDto): string {
  const w = focusWorker(root);
  if (w) return w.id;
  return root.children.find((c) => c.kind === "run")?.id ?? root.id;
}

/**
 * A run has stalled when something still says it is working but nothing is: no live process exists for the run, or the
 * run is live and no working worker's stream has grown for `STALL_MS` (and the view has watched long enough to know).
 * `canTell` is false when no working worker writes a stream (only the core's own workers are working): then a live run
 * is never called silent, since nothing here could see it work. Returns when the last activity was (ms, null when
 * unknown), or undefined when the run has not stalled.
 */
export function stalledSince(o: { working: boolean; live: boolean; canTell: boolean; lastMs: number | null; watchedMs: number; activeNow: boolean; now: number }): number | null | undefined {
  if (!o.working || o.activeNow) return undefined;
  if (!o.live) return o.lastMs;
  if (o.canTell && o.lastMs !== null && o.watchedMs >= WATCH_MS && o.now - o.lastMs >= STALL_MS) return o.lastMs;
  return undefined;
}

/** How often the Workers screen reads the head of each working worker's stream while its run is live. */
export const LIVENESS_MS = 10_000;

/**
 * Compares stream heads with the last ones seen: true when any stream grew. The first sighting of a stream only records
 * it (a head that was already there is not new activity). `seen` is updated in place.
 */
export function headsGrew(seen: Map<string, number>, heads: [string, number][]): boolean {
  let grew = false;
  for (const [id, head] of heads) {
    const was = seen.get(id);
    if (was !== undefined && head > was) grew = true;
    seen.set(id, Math.max(was ?? head, head));
  }
  return grew;
}

/** "6 Nov, 22:05" in the reader's locale. */
export function clock(ms: number): string {
  return new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(ms);
}

// ---- the stream as a feed --------------------------------------------------------------------------------------------

/** One entry of a worker's feed. Lines come from the core's transcript reader: the agent's own text has no tag, the
 * rest carry `[user] `, `[tool] `, `[result] ` or `[hook] `. Text is plain, never parsed as markup. */
export type FeedItem =
  | { kind: "say"; seq: number; text: string }
  | { kind: "prompt"; seq: number; text: string }
  | { kind: "note"; seq: number; text: string }
  | { kind: "step"; seq: number; tool: string; detail: string; result: string[] };

const TAGS: [string, "prompt" | "step" | "result" | "note"][] = [["[user] ", "prompt"], ["[tool] ", "step"], ["[result] ", "result"], ["[hook] ", "note"]];
/** Result lines kept under one step; the rest is counted, and the Raw output tab has it all. */
export const RESULT_LINES = 6;

export function feedItems(lines: readonly { seq: number; text: string }[]): FeedItem[] {
  const out: FeedItem[] = [];
  for (const l of lines) {
    const hit = TAGS.find(([p]) => l.text.startsWith(p));
    const kind = hit?.[1] ?? "say";
    // Colour and cursor codes are dropped: the feed is plain text (Raw output keeps the colours).
    const text = sanitizeLine(hit ? l.text.slice(hit[0].length) : l.text).segments.map((s) => s.text).join("");
    const last = out[out.length - 1];
    if (kind === "result") {
      // A result belongs to the step before it; one with no step in the window reads as a note.
      if (last?.kind === "step") { last.result.push(text); continue; }
      out.push({ kind: "note", seq: l.seq, text });
      continue;
    }
    if (kind === "step") {
      const sp = text.indexOf(" ");
      out.push({ kind: "step", seq: l.seq, tool: sp > 0 ? text.slice(0, sp) : text, detail: sp > 0 ? text.slice(sp + 1) : "", result: [] });
      continue;
    }
    // Consecutive lines of one kind (the agent's text, the task it was given, a message to it) are one entry.
    if (last && last.kind !== "step" && last.kind === kind) { last.text += `\n${text}`; continue; }
    out.push({ kind, seq: l.seq, text });
  }
  return out;
}

/** A short line for a screen reader about the newest entry: the step's tool, or the start of the message. */
export function announceOf(item: { kind: "step"; tool: string; detail: string } | { kind: string; text: string } | undefined): string {
  if (!item) return "";
  if ("tool" in item) return item.detail ? `${item.tool} ${item.detail}`.slice(0, 120) : item.tool;
  return item.text.split("\n")[0].slice(0, 120);
}
