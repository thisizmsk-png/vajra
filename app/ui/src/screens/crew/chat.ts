// A worker's stream read as a conversation: who said what, the steps it took, and the messages that reached it. Built on
// `feedItems` (the stream's tags), so the stream format is parsed in one place. Pure functions, tested without a screen.
import { feedItems } from "./crewLive";
import type { FeedItem } from "./crewLive";

/** Where a message sent through the core's steering path is. `seen`: its block is in the worker's own transcript, which
 * the control hook writes only when it hands the message to the worker. `ended`: the worker finished before that. */
export type Delivery = "sending" | "sent" | "seen" | "failed" | "ended";

/** One message the human sent to a worker from this app. `id` is the core's action id (empty until it answers). */
export type Sent = { key: string; id: string; text: string; redirect: boolean; state: Delivery; error?: string };

export type TestSummary = { passed: number; failed: number };

export type ChatItem =
  /** The worker's own sentence. */
  | { kind: "agent"; seq: number; text: string }
  /** The task the Lead gave the worker (the transcript's prompt). */
  | { kind: "lead"; seq: number; text: string }
  /** A message relayed to the worker by the steering hook, or one the human sent that has not reached it yet. */
  | { kind: "steer"; seq: number; from: "you" | "lead"; redirect: boolean; id: string; text: string; state?: Delivery; error?: string }
  /** Hook context that is not a relayed message. */
  | { kind: "note"; seq: number; text: string }
  | { kind: "step"; seq: number; tool: string; detail: string; path: string | null; result: string[]; test: TestSummary | null };

/** Tools whose detail is the file they touched. */
const FILE_TOOLS = new Set(["Read", "Edit", "Write", "MultiEdit", "NotebookEdit"]);
/** Tools that change the file: their change is shown from the worktree, not from the tool's own result text. */
export const WRITE_TOOLS = new Set(["Edit", "Write", "MultiEdit", "NotebookEdit"]);

const unescape = (s: string) => s.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
const attr = (head: string, name: string) => new RegExp(`\\b${name}="([^"]*)"`).exec(head)?.[1] ?? "";
const STEER = /<vajra-steer\b([^>]*)>\n?([\s\S]*?)\n?<\/vajra-steer>/g;
const REDIRECT_PREFIX = /^Change of direction from the (?:Lead|human): /;

/** The relayed messages in one hook entry, in order; an entry with none is a plain note. */
export function steerBlocks(text: string): { from: "you" | "lead"; redirect: boolean; id: string; text: string }[] {
  const out: { from: "you" | "lead"; redirect: boolean; id: string; text: string }[] = [];
  for (const m of text.matchAll(STEER)) {
    const head = m[1];
    const redirect = attr(head, "kind") === "redirect";
    const body = unescape(m[2]).replace(REDIRECT_PREFIX, "");
    out.push({ from: attr(head, "source") === "lead-via-desktop" ? "lead" : "you", redirect, id: attr(head, "id"), text: body });
  }
  return out;
}

/** Pass and fail counts from a test run's output, read from its last summary line; null when it is not one. */
export function testSummary(lines: readonly string[]): TestSummary | null {
  for (let i = lines.length - 1; i >= 0; i--) {
    const l = lines[i];
    // cargo: "test result: FAILED. 3 passed; 1 failed; ..."
    const cargo = /test result: \w+\. (\d+) passed; (\d+) failed/.exec(l);
    if (cargo) return { passed: Number(cargo[1]), failed: Number(cargo[2]) };
    if (!/\b(tests?|passed|failed|passing|failing)\b/i.test(l)) continue;
    const passed = /(\d+)\s+(?:passed|passing)\b/i.exec(l);
    const failed = /(\d+)\s+(?:failed|failing)\b/i.exec(l);
    if (passed || failed) return { passed: passed ? Number(passed[1]) : 0, failed: failed ? Number(failed[1]) : 0 };
  }
  return null;
}

/** The core writes a step as `Name: detail`; an older stream as `Name detail`. Either way the tool is the bare name. */
const toolName = (t: string) => t.replace(/:$/, "");

function toChat(f: FeedItem): ChatItem[] {
  if (f.kind === "say") return [{ kind: "agent", seq: f.seq, text: f.text }];
  if (f.kind === "prompt") return [{ kind: "lead", seq: f.seq, text: f.text }];
  if (f.kind === "note") {
    const blocks = steerBlocks(f.text);
    if (!blocks.length) return [{ kind: "note", seq: f.seq, text: f.text }];
    // Several messages in one entry share its seq; the fraction keeps their keys apart and their order.
    return blocks.map((b, i) => ({ kind: "steer", seq: f.seq + i / 100, ...b }));
  }
  const tool = toolName(f.tool);
  const path = FILE_TOOLS.has(tool) && f.detail && !f.detail.startsWith("{") ? f.detail : null;
  return [{ kind: "step", seq: f.seq, tool, detail: f.detail, path, result: f.result, test: tool === "Bash" ? testSummary(f.result) : null }];
}

/**
 * The stream as chat entries, with the human's own sent messages placed in it: one that reached the worker takes the
 * place of its relayed block (and reads Seen); one that has not yet is shown at the end with where it is.
 */
export function chatItems(lines: readonly { seq: number; text: string }[], sent: readonly Sent[] = [], finished = false): ChatItem[] {
  const items = feedItems(lines).flatMap(toChat);
  const reached = new Set(items.flatMap((i) => (i.kind === "steer" && i.id ? [i.id] : [])));
  const mine = new Map(sent.filter((s) => s.id).map((s) => [s.id, s]));
  for (const i of items) if (i.kind === "steer" && mine.has(i.id)) i.state = "seen";
  const last = items[items.length - 1]?.seq ?? 0;
  sent.forEach((s, n) => {
    if (s.id && reached.has(s.id)) return;
    const state: Delivery = s.state === "sent" && finished ? "ended" : s.state;
    items.push({ kind: "steer", seq: last + 0.5 + n / 1000, from: "you", redirect: s.redirect, id: s.id, text: s.text, state, error: s.error });
  });
  return items;
}

/** The newest line of activity, for a status that says what a worker is doing ("Running npm test", "Editing src/a.ts"). */
export function lastLine(items: readonly ChatItem[]): string {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "step") return (it.detail ? `${it.tool} ${it.detail}` : it.tool).split("\n")[0].slice(0, 140);
    if (it.kind === "agent") return it.text.split("\n").filter(Boolean).pop()?.slice(0, 140) ?? "";
  }
  return "";
}

/** One file's part of a unified diff, or "" when the diff does not touch it. `path` may be absolute (a tool's
 * `file_path`); the diff names files relative to the repository, so the match is on the path's end. */
export function fileDiff(diff: string, path: string): string {
  const parts = diff.split(/^(?=diff --git )/m);
  const ends = (name: string) => name !== "" && (path === name || path.endsWith(`/${name}`));
  for (const p of parts) {
    const m = /^diff --git a\/(.+?) b\/(.+?)$/m.exec(p);
    if (m && (ends(m[1]) || ends(m[2]))) return p;
  }
  return "";
}
