// What the welcome shows, worked out from what the app knows. Pure: no calls, so each state in the design is one test.
import type { CheckKind } from "./CheckRow";
import type { LoginState } from "../providerContract";
import type { SetupRow } from "../setupContract";
import { S } from "../../strings";

const C = S.welcome;

export type Repo = { root: string; name: string; worktrees: number };
export type Repos =
  | { at: "loading" }
  | { at: "found"; list: Repo[]; truncated: boolean }
  | { at: "none" }
  | { at: "failed"; detail: string };
export type Picked = { id: string; name: string; root: string } | null;
/** git and Python 3, the two things Apple's developer tools bring. `unknown` when the checks did not answer: that never blocks. */
export type Tools = "ok" | "missing" | "unknown";
/** A read that has not answered yet, or failed, is its own state: it is never shown as "none installed". */
export type Read<T> = T | "loading" | "failed";

export interface AgentInfo { row: SetupRow; login: LoginState }
export interface Check { kind: CheckKind; word: string; summary: string }
export interface Model {
  folder: Check;
  agent: Check;
  engine: Check;
  ready: number;
  /** The row that needs you first (1 folder, 2 agent, 3 engine); 4 when every check is ready and the ask is next. */
  first: 1 | 2 | 3 | 4;
  agentOk: boolean;
  toolsOk: boolean;
  engineOk: boolean;
}

/** The core's own words for what went wrong, never its error code (the design: no raw "claude_dir_missing"). Empty when it gave none. */
export const reason = (e: unknown): string => {
  const d = typeof e === "object" && e !== null ? (e as { detail?: unknown }).detail : undefined;
  return typeof d === "string" ? d.trim() : "";
};

/** An agent counts when it is installed and its own login check says signed in. "Cannot tell" never counts. */
export const agentReady = (a: AgentInfo): boolean => a.row.state === "installed" && a.login === "signed_in";
/** Installed but signed out is the one state a click fixes. */
export const agentSignedOut = (a: AgentInfo): boolean => a.row.state === "installed" && a.login === "signed_out";

export function derive(i: { repos: Repos; picked: Picked; tools: Tools; agents: Read<AgentInfo[]>; engine: Read<boolean> }): Model {
  const list = Array.isArray(i.agents) ? i.agents : null;
  const toolsOk = i.tools !== "missing";
  const ready = list?.find(agentReady);
  const out = list?.find(agentSignedOut);
  const agentOk = !!ready;
  const engineOk = i.engine === true;
  const proj = !!i.picked;
  const first: Model["first"] = !proj ? 1 : !agentOk || !toolsOk ? 2 : !engineOk ? 3 : 4;
  const todo = (summary: string): Check => ({ kind: "todo", word: C.word.todo, summary });
  const need = (summary: string): Check => ({ kind: "warn", word: C.word.warn, summary });

  const folder: Check = proj
    ? { kind: "pass", word: C.word.pass, summary: C.folder.pickedSummary(i.picked!.name, i.picked!.root) }
    : need(i.repos.at === "found" ? C.folder.found(i.repos.list.length) : i.repos.at === "none" ? C.folder.none : i.repos.at === "failed" ? C.folder.failed : C.folder.looking);

  const agent: Check = toolsOk && agentOk ? { kind: "pass", word: C.word.pass, summary: C.agent.readySummary(ready!.row.name, ready!.row.version) }
    : !toolsOk ? todo(C.agent.toolsSummary)
    : i.agents === "loading" ? todo(C.checking)
    : i.agents === "failed" ? todo(C.agent.failedSummary)
    : out ? need(C.agent.signedOutSummary(out.row.name))
    : todo(C.agent.noneSummary);

  const engine: Check = engineOk ? { kind: "pass", word: C.word.pass, summary: C.engine.readySummary }
    : i.engine === "loading" ? todo(C.checking)
    : i.engine === "failed" ? todo(C.agent.failedSummary)
    : first === 3 ? need(C.engine.missingSummary) : todo(C.engine.missingSummary);

  const count = [proj, toolsOk && agentOk, engineOk].filter(Boolean).length;
  return { folder, agent, engine, ready: count, first, agentOk, toolsOk, engineOk };
}
