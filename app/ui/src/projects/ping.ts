// The ping rule at work (the attention budget). The core tells the UI that something new started waiting
// (`inbox_new {project_id, kind, count}`, already coalesced over 5 s); this decides whether that becomes a ping now,
// joins one that is about to leave, waits for the hourly budget, or stays quiet. Every outcome carries why.
//
// It never sees an item's title or body (untrusted, maybe secret), only the project, the kind and a count. It can only
// show or hold a notice: it cannot start, approve or dismiss anything, and every item stays in the Needs-you queue
// whatever it decides. Pure and clock-free so tests pin it; PingHost owns the one timer.
import { S } from "../../strings";
import type { PingRule } from "../preferenceContract";

export const HOUR_MS = 60 * 60_000;
/** The rule's three switches. Plans and Director-General proposals are decisions; a blocked or failed task is blocked. */
export type PingGroup = "decisions" | "questions" | "blocked";
export const PING_GROUPS: readonly PingGroup[] = ["decisions", "questions", "blocked"];

/** The core's arrival kinds (a task arrives by its status). Anything else is not a Needs-you item and never pings. */
export function groupOf(kind: string): PingGroup | null {
  if (kind === "plan" || kind === "run_proposal") return "decisions";
  if (kind === "question") return "questions";
  if (kind === "blocked" || kind === "failed") return "blocked";
  return null;
}

export type Arrival = { project_id: string; kind: string; count: number };
/** One line of the ping log: a ping that left (`pinged`), or an arrival the rule kept quiet. */
export type PingEntry = { at: number; pinged: boolean; text: string; why: string };
type Waiting = { project: string; group: PingGroup; count: number };
export type PingState = {
  /** When each ping of the last hour left, oldest first. */
  sent: number[];
  /** What the next ping will tell, by project and group, in arrival order. */
  waiting: Waiting[];
  /** When the next ping is due; null when nothing waits. */
  due: number | null;
  /** Set once a due ping found the hour's budget spent. */
  held: boolean;
};
export const EMPTY_PING_STATE: PingState = { sent: [], waiting: [], due: null, held: false };

type Names = (projectId: string) => string;

/** What a set of arrivals says: the one thing, or how many and where. Counts and project names only. */
export function pingText(waiting: Waiting[], name: Names): string {
  const total = waiting.reduce((n, w) => n + w.count, 0);
  if (waiting.length === 1 && total === 1) return S.ping.one[waiting[0].group](name(waiting[0].project));
  const byProject = new Map<string, [PingGroup, number][]>();
  for (const w of waiting) byProject.set(w.project, [...(byProject.get(w.project) ?? []), [w.group, w.count]]);
  return S.ping.many(total, [...byProject].map(([p, counts]) => S.ping.inProject(counts, name(p))));
}

/** Why a ping left: which switches let it through, and whether it was grouped or held by the budget. */
export function pingWhy(waiting: Waiting[], rule: PingRule, held: boolean): string {
  const groups = PING_GROUPS.filter((g) => waiting.some((w) => w.group === g));
  const total = waiting.reduce((n, w) => n + w.count, 0);
  const parts = [S.ping.why.rule(groups)];
  if (total > 1 && rule.batch_minutes > 0) parts.push(S.ping.why.grouped(rule.batch_minutes));
  if (held) parts.push(S.ping.why.held(rule.per_hour));
  return parts.join(" ");
}

/**
 * One `inbox_new`. A kind the rule turns off, or notifications turned off, is logged quietly with the reason; anything
 * else waits for the next ping, which is due `batch_minutes` after the first arrival that opened it.
 */
export function arrive(s: PingState, a: Arrival, now: number, rule: PingRule, notify: boolean, name: Names): { state: PingState; quiet: PingEntry | null } {
  const group = groupOf(a.kind);
  const count = Math.max(1, Math.floor(Number(a.count)) || 1);
  if (!group) return { state: s, quiet: null };
  const one = [{ project: a.project_id, group, count }];
  if (!notify) return { state: s, quiet: { at: now, pinged: false, text: pingText(one, name), why: S.ping.why.off } };
  if (!rule[group]) return { state: s, quiet: { at: now, pinged: false, text: pingText(one, name), why: S.ping.why.muted(group) } };
  const at = s.waiting.findIndex((w) => w.project === a.project_id && w.group === group);
  const waiting = at < 0 ? [...s.waiting, one[0]] : s.waiting.map((w, i) => (i === at ? { ...w, count: w.count + count } : w));
  return { state: { ...s, waiting, due: s.due ?? now + rule.batch_minutes * 60_000 }, quiet: null };
}

/**
 * Called when the timer fires. Not due yet: nothing. Due with budget left: one ping for everything waiting. Due with
 * the hour's budget spent: it waits until the oldest ping of the hour ages out, and then says it was held.
 */
export function flush(s: PingState, now: number, rule: PingRule, name: Names): { state: PingState; ping: PingEntry | null } {
  if (s.due === null || now < s.due || s.waiting.length === 0) return { state: s.waiting.length === 0 ? { ...s, due: null } : s, ping: null };
  const sent = s.sent.filter((t) => t > now - HOUR_MS);
  if (sent.length >= rule.per_hour) return { state: { ...s, sent, due: sent[sent.length - rule.per_hour] + HOUR_MS, held: true }, ping: null };
  const ping = { at: now, pinged: true, text: pingText(s.waiting, name), why: pingWhy(s.waiting, rule, s.held) };
  return { state: { sent: [...sent, now], waiting: [], due: null, held: false }, ping };
}
