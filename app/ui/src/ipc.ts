// The one place that names the core's IPC commands. Screens call `api.*`, never `invoke`, so a screen can be tested
// against `setTransport(fake)` and a command rename is a one-file change. Argument names are camelCase here; Tauri maps
// them to the snake_case parameters of the Rust commands.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppInfo, Project, Session } from "./projectContract";

/** What a failed command rejects with: `{error: code, detail?}` from the core (a control error adds its own fields). */
export type IpcError = { error: string; detail?: string; [k: string]: unknown };
export const isIpcError = (e: unknown): e is IpcError => typeof e === "object" && e !== null && typeof (e as IpcError).error === "string";

/** A short line safe to show: the code and detail from the core, or a plain fallback. Never a stack. */
export function errorText(e: unknown): string {
  // The Lead's guard preconditions already carry a one-line "why and what to do" (for example the sandbox one), so
  // it is shown as it is, without the code in front.
  if (isIpcError(e) && e.error === "lead_guard_missing" && e.detail) return e.detail;
  if (isIpcError(e)) return e.detail ? `${e.error}: ${e.detail}` : e.error;
  return "unexpected error";
}

export interface Transport {
  call(cmd: string, args?: Record<string, unknown>): Promise<unknown>;
  /** Subscribe to a core event; returns the unsubscribe function. */
  on(event: string, cb: (payload: unknown) => void): () => void;
}

const tauri: Transport = {
  call: (cmd, args) => invoke(cmd, args),
  on(event, cb) {
    let off: (() => void) | null = null;
    let dead = false;
    void listen(event, (e) => cb(e.payload)).then((u) => (dead ? u() : (off = u)));
    return () => {
      dead = true;
      off?.();
    };
  },
};

let transport: Transport = tauri;
/** Tests install a fake; pass nothing to restore the real transport. */
export const setTransport = (t?: Transport) => void (transport = t ?? tauri);

const call = <T>(cmd: string, args?: Record<string, unknown>) => transport.call(cmd, args) as Promise<T>;

// ---- response shapes (the JSON the core returns) --------------------------------------------------------------------
export type PreflightRow = { name: string; ok: boolean; detail: string; fix: string };
export type Preflight = { rows: PreflightRow[]; start_enabled: boolean };
export type Campaign = {
  slug: string; goal?: string; status?: string; created?: string; updated?: string; passing?: number; total?: number; error?: string;
  /** The signature check: ok, failed or unknown (could not run). */
  /** "checking": the list came back before this campaign's verify ran; ask `campaignIntegrity`. */
  integrity?: "ok" | "failed" | "unknown" | "checking";
};
export type ReportIntegrity = "ok" | "no_report" | "integrity_failed" | "not_text" | "unknown";
export type Report = { slug: string; integrity: ReportIntegrity; text?: string };
export type Verified = { slug: string; ok: boolean; restore?: string | null };
export type RunHandle = { stream: string; session_id: string; run_id: string; campaign: string };
export type CrewNodeDto = { id: string; kind: string; title: string; engine_id: string | null; role?: string | null; status: string; overlays: unknown[]; children: CrewNodeDto[] };
export type Line = { seq: number; text: string };
export type Tail = { lines: Line[]; gap: boolean; head_seq: number; spill_start: number | null; spill_end: number | null };
/** `stream_window`: only the rows a view shows. `head_seq - first_seq + 1` is the row count the scrollbar stands for. */
export type StreamWindow = { lines: Line[]; head_seq: number; first_seq: number; at_start: boolean; at_end: boolean; gap: boolean };
export type ResumeTarget = { session_id: string; run_id: string; cwd: string };
export type ActiveRuns = { count: number; runs: unknown[] };
export type Json = Record<string, unknown>;
export type EvidenceStatus = "pass" | "fail" | "not_run" | "unverified" | "checking" | "stale";
export type EvidenceRow = {
  task: string; title: string | null; owner: string | null; criterion: string; status: EvidenceStatus;
  head: string | null; path: string | null; commands: number | null; report: string | null;
};
/** One recorded verification command with its exit code; `output_tail` is the end of its output for a failed one. Plain text. */
export type ProofCommand = { cmd: string | null; exit_code: number | null; passed: boolean; output_tail: string | null };
export type TimelinePhase = { name: string; state: "done" | "current" | "pending"; round?: number | null; max_rounds?: number | null };
export type GraphTask = { id: string; local: string; title: string | null; owner: string | null; status: string; deps: string[]; wave: number };
export type RunOverview = {
  slug: string; run_id: string; stale: boolean;
  timeline: { phases: TimelinePhase[]; current: string; status: "active" | "done" | "need_input" | "rejected" };
  graph: { tasks: GraphTask[]; waves: number };
};
export type EvidenceDetail = { task: string; commands: ProofCommand[]; notes: string | null; truncated: boolean };
export type WorktreeInfo = { worker: string; worktree: string; branch: string | null; base: string | null };
export type AskStarted = { ask_id: string; state: string; stream: string; lead_session_id: string };
export type LeadStatus = { available: boolean; read_only_reason?: string; busy: boolean; ask_id?: string };
/** ADR-14 (read-only, B1): a task waiting on the human. `body` and `context` are untrusted text, shown as plain text. */
export type InboxItem = {
  id: string; kind: "task" | "plan" | "question"; campaign: string; origin: "app" | "terminal"; status?: string; created: string; age_s: number;
  title: string; body: string; context?: string[]; actions: string[];
  /** B2: plan/question items come from the workflow's own result file, which is unsigned. Show this label with them. */
  source?: string;
  /** B2 plan items: what `approve_plan` must be given back, the hash of exactly what was shown. */
  plan_sha256?: string; action?: string;
  /** The deploy plan as the workflow wrote it (`S.DeployPlan`), unabridged; unsafe characters appear as visible `\u{..}` escapes. */
  plan?: { target: string; commands: string[]; rollback: string[]; health_checks: string[]; risk: "low" | "medium" | "high" } | null;
  /** The whole pending_approval entry, unabridged: what `plan_sha256` covers. */
  entry?: Record<string, unknown>;
  /** Set when the entry cannot be approved (only `reject` is offered). */
  invalid?: string | null;
  /** What `created` and `age_s` measure: when the app first saw the item waiting (tasks.json has no per-task time). */
  since?: string;
};
export type RunInput = { ask: string; maxRounds: number; base: string; maxInFlight: number; campaign: string | null };

export const api = {
  preflight: () => call<Preflight>("preflight"),
  listCampaigns: (refresh = false) => call<{ campaigns: Campaign[] }>("list_campaigns", { refresh }),
  /** The signed delivery report, verified on the bytes the core read; `text` only when integrity is "ok". */
  readReport: (slug: string) => call<Report>("read_report", { slug }),
  /** Run the signature check now (not the cached badge). */
  verifyCampaign: (slug: string) => call<Verified>("verify_campaign", { slug }),
  /** One campaign's badge (#28): `ok`, `failed` or `unknown`. The core runs at most four verifies at once. */
  campaignIntegrity: (slug: string, refresh = false) => call<{ slug: string; integrity: "ok" | "failed" | "unknown" }>("campaign_integrity", { slug, refresh }),
  startRun: (i: RunInput, cwd: string) => call<RunHandle>("start_run", { ...i, cwd }),
  resumeRun: (sessionId: string, campaign: string, cwd: string) => call<RunHandle>("resume_run", { sessionId, campaign, cwd }),
  crew: (slug: string, runId: string) => call<CrewNodeDto>("crew", { slug, runId }),
  tail: (stream: string, afterSeq: number, maxBytes = 65536) => call<Tail>("tail", { stream, afterSeq, maxBytes }),
  /** The lines just before `beforeSeq` (at most `maxLines`), for Load earlier; `at_start` when nothing older exists. */
  readSpillBefore: (stream: string, beforeSeq: number, maxLines: number) => call<{ lines: Line[]; at_start: boolean; gap: boolean }>("read_spill_before", { stream, beforeSeq, maxLines }),
  /** Up to `maxLines` lines starting EXACTLY at seq `from` (clamped to the oldest kept line, then `gap`), or the newest when `from` is null. */
  streamWindow: (stream: string, from: number | null, maxLines: number, maxBytes = 1048576) => call<StreamWindow>("stream_window", { stream, from, maxLines, maxBytes }),
  readSpill: (stream: string, offset: number, maxBytes = 65536) => call<Json>("read_spill", { stream, offset, maxBytes }),
  task: (slug: string, taskId: string) => call<Json>("task", { slug, taskId }),
  /** A worker's crew worktree path, branch and diff base; refused for any directory outside the crew worktrees. */
  worktreeInfo: (worker: string) => call<WorktreeInfo>("worktree_info", { worker }),
  /** Every acceptance criterion of a campaign with its proof status (bounded checks on the core side). */
  evidenceList: (slug: string) => call<{ slug: string; stale: boolean; pending?: number; rows: EvidenceRow[] }>("evidence_list", { slug }),
  runOverview: (slug: string) => call<RunOverview>("run_overview", { slug }),
  evidenceDetail: (slug: string, taskId: string) => call<EvidenceDetail>("evidence_detail", { slug, taskId }),
  diff: (cwd: string, base: string) => call<{ code: number | null; diff: string; truncated: boolean; timed_out: boolean; untracked?: string[]; untracked_truncated?: boolean }>("diff", { cwd, base }),
  controlMessage: (worker: string, text: string) => call<Json>("control_message", { worker, text }),
  controlRedirect: (worker: string, text: string) => call<Json>("control_redirect", { worker, text }),
  controlStop: (worker: string, reason?: string) => call<Json>("control_stop", { worker, reason: reason ?? null }),
  confirmStop: (actionId: string) => call<Json>("confirm_stop", { actionId }),
  cancelStop: (actionId: string) => call<Json>("cancel_stop", { actionId }),
  resumeWorker: (worker: string) => call<Json>("resume_worker", { worker }),
  ledger: () => call<{ records: Json[] }>("ledger"),
  kbSearch: (query: string, k: number, repo: string) => call<{ hits: unknown; code: number | null }>("kb_search", { query, k, repo }),
  takeOver: (stream: string, on: boolean) => call<Json>("take_over", { stream, on }),
  /** The session and directory of each campaign this app started (absent: started elsewhere). */
  resumeTargets: () => call<{ targets: Record<string, ResumeTarget> }>("resume_targets"),
  activeRuns: () => call<ActiveRuns>("active_runs"),
  /** ADR-13: one headless Lead turn per question. The reply streams into `stream` (`lead-<run_id>`). */
  askLead: (runId: string, text: string) => call<AskStarted>("ask_lead", { runId, text }),
  cancelAsk: (askId: string) => call<Json>("cancel_ask", { askId }),
  leadStatus: (runId: string) => call<LeadStatus>("lead_status", { runId }),
  /** Everything waiting on the human, oldest first. Derived on each call; poll it like `crew`. */
  /** Approve plan `id` as shown: starts the approved action as a new turn. Refused (`plan_changed`) if the plan changed since `inbox` showed it. */
  approvePlan: (id: string, planSha256: string) => call<{ action_id: string; state: string; via: string }>("approve_plan", { id, planSha256 }),
  /** Reject plan `id` with a reason (1 to 500 characters). */
  rejectPlan: (id: string, reason: string) => call<{ action_id: string; state: string }>("reject_plan", { id, reason }),
  inbox: () => call<{ count: number; items: InboxItem[] }>("inbox"),
  /** Which record versions this build reads (`vajra.app_info/1`). */
  appInfo: () => call<AppInfo>("app_info"),
  /** Active projects, oldest first; `skipped` counts records that were damaged, foreign or from a newer version. */
  projectList: (includeArchived = false) => call<{ projects: Project[]; skipped: number }>("project_list", { includeArchived }),
  /** Add a folder. Fails `nested` (with `with: {id, name}`) when it overlaps a project unless `confirmNested`; `refused_folder` for home, the Claude folder or Vajra's data. */
  projectAdd: (path: string, confirmNested = false) =>
    call<{ outcome: "created" | "existing" | "restored"; project: Project }>("project_add", { path, confirmNested }),
  /** Fails `conflict` (with `current_rev`) when `expectRev` is stale. */
  projectRename: (id: string, name: string, expectRev?: number) => call<{ project: Project }>("project_rename", { id, name, expectRev }),
  projectArchive: (id: string, expectRev?: number) => call<{ project: Project }>("project_archive", { id, expectRev }),
  /** `skipped` counts unusable lines; `unsupported` means a newer build wrote records this one cannot read, so show "update needed". */
  projectSessions: (id: string) => call<{ sessions: Session[]; skipped: number; unsupported: boolean }>("project_sessions", { id }),
};

/** The core says something changed under the Vajra state dir; payload is the list of changed campaign slugs. */
export const onChanged = (cb: (payload: unknown) => void) => transport.on("changed", cb);
