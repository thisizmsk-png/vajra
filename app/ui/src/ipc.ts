import type { PreferencePatch, PreferenceView, PreferenceEnvelope, PreferenceActResult } from "./preferenceContract";
// The one place that names the core's IPC commands. Screens call `api.*`, never `invoke`, so a screen can be tested
// against `setTransport(fake)` and a command rename is a one-file change. Argument names are camelCase here; Tauri maps
// them to the snake_case parameters of the Rust commands.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ProviderList, UsageList } from "./providerContract";
import type { ProposalOverride } from "./accountsContract";
import type { AppInfo, Project, Session } from "./projectContract";

/** What a failed command rejects with: `{error: code, detail?}` from the core (a control error adds its own fields). */
export type IpcError = { error: string; detail?: string; [k: string]: unknown };
export const isIpcError = (e: unknown): e is IpcError => typeof e === "object" && e !== null && typeof (e as IpcError).error === "string";

const PLAIN = new Set(["agents_off", "lead_guard_missing", "turns_full", "not_a_project", "project_archived", "project_missing"]);

/** A short line safe to show: the code and detail from the core, or a plain fallback. Never a stack. */
export function errorText(e: unknown): string {
  // A refused projects folder (#77 follow-up): what is wrong, then what to do (the fix names the path).
  if (isIpcError(e) && e.error === "projects_unavailable" && e.detail) {
    const fix = (e as { fix?: unknown }).fix;
    return typeof fix === "string" && fix ? `${e.detail} ${fix}` : e.detail;
  }
  // The Director-General's guard preconditions, its turn cap and the project-store refusals (FG-02c) already carry a
  // one-line "why and what to do" (for example the sandbox one), so they are shown as they are, without the code.
  if (isIpcError(e) && PLAIN.has(e.error) && e.detail) return e.detail;
  if (isIpcError(e)) return e.detail ? `${e.error}: ${e.detail}` : e.error;
  return "unexpected error";
}

export interface Transport {
  call(cmd: string, args?: Record<string, unknown>): Promise<unknown>;
  /** Subscribe to a core event; returns the unsubscribe function. */
  on(event: string, cb: (payload: unknown) => void): () => void;
  /** Like `call`, for a command that streams to a Tauri `Channel`: `onMessage` gets each channel message (raw output
   *  bytes, or the JSON `{exit}`). A transport without it cannot open terminals. */
  channel?(cmd: string, args: Record<string, unknown>, onMessage: (m: unknown) => void): Promise<unknown>;
}

const tauri: Transport = {
  call: (cmd, args) => invoke(cmd, args),
  channel(cmd, args, onMessage) {
    const ch = new Channel<unknown>();
    ch.onmessage = onMessage;
    return invoke(cmd, { ...args, onEvent: ch });
  },
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

function termStream(cmd: string, args: Record<string, unknown>, onMsg: (m: TermMsg) => void): Promise<TermInfo> {
  if (!transport.channel) return Promise.reject({ error: "no_terminal_transport", detail: "this backend cannot stream" });
  const bytes = (m: unknown): Uint8Array | null =>
    Object.prototype.toString.call(m) === "[object ArrayBuffer]" ? new Uint8Array(m as ArrayBuffer) : ArrayBuffer.isView(m) ? new Uint8Array(m.buffer, m.byteOffset, m.byteLength) : Array.isArray(m) ? Uint8Array.from(m as number[]) : null;
  return transport.channel(cmd, args, (m) => {
    const b = bytes(m);
    if (b) onMsg({ data: b });
    else if (typeof m === "object" && m !== null && "exit" in m) onMsg({ exit: (m as { exit: number | null }).exit });
    else if (typeof m === "object" && m !== null && "skipped" in m) {
      const r = m as { skipped: unknown; alt?: unknown };
      onMsg({ skipped: Number(r.skipped) || 0, alt: typeof r.alt === "boolean" ? r.alt : null });
    }
    else if (typeof m === "object" && m !== null && "activity" in m) onMsg({ activity: true });
  }) as Promise<TermInfo>;
}

// ---- response shapes (the JSON the core returns) --------------------------------------------------------------------
/** `action`: a one-click fix the core offers (today only `install_dev_tools`), with its label and what to do after. */
export type PreflightRow = { name: string; ok: boolean; detail: string; fix: string; action?: "install_dev_tools"; action_label?: string; action_note?: string };
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
export type CrewNodeDto = { id: string; kind: string; title: string; engine_id: string | null; role?: string | null; /** The provider this worker ran on (from the launch receipt's `effective`); parse with `parseWorkerProvider`. */ provider?: unknown; /** The worker's change set; parse with `parseWorkerDiff`. */ diff?: unknown; status: string; overlays: unknown[]; children: CrewNodeDto[] };
/** What `term_open`, `term_attach` and `term_list` say about a user terminal. `alive` is false once the shell has ended. */
export type TermInfo = { id: number; cwd: string; root: string; rows: number; cols: number; alive: boolean };
/** `lifecycle`: what keeps running after the window closes, after a quit and after an archive (the core computes it). */
export type TermDiag = { id: number; pid: number; cwd: string; root: string; alive: boolean; in_worktree: boolean };
export type RunDiag = { stream: string; pid: number };
/** A shell an earlier session recorded, checked at launch by pid and start time. Only `owned` can be ended. */
export type Found = { pid: number; stamp: number; cwd: string; verdict: "owned" | "gone" | "reused"; members?: number };
export type Lifecycle = { terminals: TermDiag[]; runs: RunDiag[]; found: Found[]; close_keeps: { terminals: number; runs: number }; quit_ends: { terminals: number; runs: number }; archive_leaves: number[] };
/** One message on a terminal's channel: output bytes, or the end of the shell (`code` null when a signal ended it). */
export type TermMsg =
  | { data: Uint8Array }
  | { exit: number | null }
  /** A hidden pane was shown: `skipped` bytes of its output are not coming; `alt` says it entered (true) or left (false) the alternate screen meanwhile. */
  | { skipped: number; alt: boolean | null }
  /** New output arrived in a pane that is hidden (the core sends it at most twice a second). */
  | { activity: true };
/** The largest `term_write` the core accepts is 256 KiB; the UI sends pieces far below it. */
export const TERM_WRITE_CHUNK = 32 * 1024;

export type Line = { seq: number; text: string };
export type Tail = { lines: Line[]; gap: boolean; head_seq: number; spill_start: number | null; spill_end: number | null };
/** `stream_window`: only the rows a view shows. `head_seq - first_seq + 1` is the row count the scrollbar stands for. */
export type StreamWindow = { lines: Line[]; head_seq: number; first_seq: number; at_start: boolean; at_end: boolean; gap: boolean };
export type ResumeTarget = { session_id: string; run_id: string; cwd: string; /** The adapter that ran it; absent reads as claude. */ provider?: string };
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
export type GraphTask = { id: string; local: string; title: string | null; owner: string | null; status: string; deps: string[]; wave: number; provider?: unknown };
export type RunOverview = {
  slug: string; run_id: string; stale: boolean;
  timeline: { phases: TimelinePhase[]; current: string; status: "active" | "done" | "need_input" | "rejected" };
  graph: { tasks: GraphTask[]; waves: number };
};
export type EvidenceDetail = { task: string; commands: ProofCommand[]; notes: string | null; truncated: boolean };
export type WorktreeInfo = { worker: string; worktree: string; branch: string | null; base: string | null };
/** ADR-15 2: one Director-General per project; `generation` is its conversation number (1, then +1 per "New conversation"). */
export type AskStarted = { ask_id: string; state: string; stream: string; lead_session_id: string; project_id: string; project_name?: string | null; generation: number; name: string };
export type DirectorGeneralState = "idle" | "thinking" | "running";
export type LeadStatus = {
  available: boolean; read_only_reason?: string; busy: boolean; ask_id?: string; name: string;
  state?: DirectorGeneralState; reason?: string; project_id?: string; generation?: number; stream?: string;
  /** The store project's name (FG-02c). */
  project_name?: string | null;
  /** When unavailable because of the project store: `not_a_project`, `project_archived` or `project_missing`. For
   *  `not_a_project`, `action: "add_project"` and the folder `path` let the window offer to add it (decision Q1). */
  error?: "not_a_project" | "project_archived" | "project_missing"; action?: "add_project"; path?: string;
};
export type NewConversation = { project_id: string; generation: number; stream: string };
/** ADR-14 (read-only, B1): a task waiting on the human. `body` and `context` are untrusted text, shown as plain text. */
export type InboxItem = {
  id: string; kind: "task" | "plan" | "question" | "run_proposal"; campaign?: string;
  origin: "app" | "terminal"; status?: string; created: string; age_s: number;
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
  /** ADR-15 2.4 `run_proposal` items: the exact run a Director-General proposed. `ask` is what starts, byte for byte. */
  ask?: string; base?: string; rounds?: number;
  /** The project this item belongs to (ADR-15 §2.6): the store's id of the project that owns the run's folder; `p_unsorted` for a run no project owns. */
  project_id?: string;
  /** `run_proposal` only: the Director-General's own id for the project (`k_` plus 32 hex). Never a project-store id. */
  orchestrator_id?: string;
  /** The project's full recorded root: what the UI matches projects on. `project_name` is only its label. */
  project?: string | null; project_name?: string | null; generation?: number;
  /** What `start_proposal` must be given back: the hash of exactly what was shown. */
  proposal_sha256?: string;
  /** `run_proposal` with worker picks (ADR-16 16.1): the seats, the security warning and the label the run will get. */
  seats_card?: SeatsCard;
};
export type SeatEntry = { seat: string; role: string; account: string; harness: "claude" | "codex" | null; network: boolean; protection: string;
  limits: { message?: boolean; redirect?: boolean; stop?: string; note?: string }; disclosures: string[];
  slot?: string; task?: string; model?: string | null; effort?: string | null; security?: boolean; why?: string; source?: string };
export type SeatsCard = { max_in_flight: number; seats: SeatEntry[]; security_warning?: string;
  evidence: { seat: string | null; trust: "guarded" | "outside_guard"; card?: string; label?: string; why?: string; approval_offer?: string } };
/** What a start with seats returns besides `run` (ADR-16 16.3): one entry per seat, and the run's evidence trust. */
export type StartReceiptSeat = { seat: string; role: string; source: string; protection: string;
  requested: { account: string; harness: string | null; model: string | null; effort: string | null; network: boolean } };
export type RunInput = { ask: string; maxRounds: number; base: string; maxInFlight: number; campaign: string | null;
  /** The effective `agent` preference (`claude` or `codex`) read when the form opened; omitted means claude. A provider that cannot start is refused, never swapped. */
  provider?: string | null;
};

/** `vajra.receipt/1` (ADR-15 section 3): the answer to every guarded action. `accepted` is never `done`. */
export type ReceiptState = "accepted" | "done" | "rejected" | "failed" | "unknown";
export type Receipt = {
  schema: "vajra.receipt/1";
  action_id: string;
  project_id: string;
  action: string;
  target: string;
  expect_generation: number | null;
  current_generation: number | null;
  state: ReceiptState;
  reason: string | null;
  detail: string | null;
  /** `safe`: retry with a NEW action_id; `after_refresh`: read project_state first; `no`: nothing to do. */
  retry: "no" | "safe" | "after_refresh";
  rev: number;
  ts_ms: number;
};
/** What a guarded action returns. `receipt` is null only when the core could not write it (nothing ran). */
export type ActResult<T = Json> = { receipt: Receipt | null; replayed?: boolean; result?: T; error?: IpcError };
/** Sent with every guarded action. `expect_generation` is the target's generation when the UI looked. */
export type CloseoutState = "closed" | "closable" | "working" | "waiting_on_you" | "unsaved_work" | "terminal_busy" | "no_worktree" | "git_unreadable";
export type CloseoutRow = { worker: string; state: CloseoutState; changed_files?: number; unpushed_commits?: number; no_upstream?: boolean; busy_terminals?: number };
export type CloseFinishedResult = { closed: { worker: string; label: string }[]; skipped: { worker: string; label: string; reason: string; changed_files?: number; unpushed_commits?: number; no_upstream?: boolean; busy_terminals?: number }[] };
/** Settings: the worker copies on disk (`worker_disk`). `bytes` is allocated space, so shared clones count in full. */
export type WorkerDisk = { copies: number; bytes: number; partial?: boolean };
export type CleanUpResult = { removed: string[]; disk: WorkerDisk };
export type Envelope = { action_id: string; project_id: string; expect_generation: number | null };
/** `vajra.project_state/1`: the one revisioned projection every surface reads. */
export type ProjectState =
  | { schema: "vajra.project_state/1"; project_id: string; rev: number; unchanged: true }
  | {
      schema: "vajra.project_state/1";
      project_id: string;
      rev: number;
      unchanged: false;
      active_runs: number;
      /** `orchestrator`: the project's Director-General conversation (FG-02c), null before its first use. */
      generations: { project: number; orchestrator?: number | null };
      workers: Json[];
      receipts: Receipt[];
      /** ADR-15 4.2 board cards, computed by the core from cached verdicts (null for a non-store id). */
      board: CoreBoard | null;
      /** This project's waiting items (at most 50; `count` is the true number). `ok: false` means the read failed: never show it as empty. */
      inbox: { ok: true; count: number; campaign_count?: number; items: InboxItem[] } | { ok: false; error?: string };
      /** This project's pending run proposals, newest first, picked before any inbox cut (at most 50; `more` were left out). */
      proposals?: { ok: true; count: number; more: number; items: InboxItem[] } | { ok: false; error?: string };
      /** The last usage answers (`vajra.provider_usage/1`), a cache read on the core side. Parse with `parseUsage`. */
      usage?: UsageList;
    };
/** One board card as the core serves it (contract (b) of the board PR). Lanes: (c); Done only when verified. */
export type CoreCard = {
  key: string; kind: "task" | "plan" | "question"; local: string; title: string; owner: string | null; status: string;
  lane: "backlog" | "running" | "review" | "done"; needs_you: boolean; criteria: { pass: number; total: number } | null;
  project_id: string; campaign: string; task_id: string | null; attempt: number | null;
};
/** The project's newest campaigns (at most 5) and their cards (at most 200 in all); `truncated` says a level was cut. */
export type CoreBoard = { campaigns: { campaign: string; run_id: string; cards: CoreCard[]; truncated: boolean }[]; truncated: boolean; error?: string };
/** The `changed` event payload: which streams moved, and the new revision of each known project. */
export type Changed = { streams: string[]; snapshot_needed: boolean; projects?: { project_id: string; rev: number }[] };

/** A fresh action id (8..=64 of A-Z a-z 0-9 - _). One per user intent; reuse it only to retry the SAME click. */
export function newActionId(): string {
  return `act-${globalThis.crypto.randomUUID()}`;
}

export const api = {
  preferencesRead: (projectId: string | null) => call<PreferenceView>("preferences_read", { projectId }),
  preferencesAct: (envelope: PreferenceEnvelope, patch: PreferencePatch, reset = false) => call<PreferenceActResult>("act_preferences_update", { envelope, patch, reset }),
  preflight: () => call<Preflight>("preflight"),
  listCampaigns: (refresh = false) => call<{ campaigns: Campaign[] }>("list_campaigns", { refresh }),
  /** The signed delivery report, verified on the bytes the core read; `text` only when integrity is "ok". */
  readReport: (slug: string) => call<Report>("read_report", { slug }),
  /** Run the signature check now (not the cached badge). */
  verifyCampaign: (slug: string) => call<Verified>("verify_campaign", { slug }),
  /** One campaign's badge (#28): `ok`, `failed` or `unknown`. The core runs at most four verifies at once. */
  campaignIntegrity: (slug: string, refresh = false) => call<{ slug: string; integrity: "ok" | "failed" | "unknown" }>("campaign_integrity", { slug, refresh }),
  startRun: (i: RunInput, cwd: string) => call<RunHandle>("start_run", { ...i, cwd }),
  /** Guarded start run (FG-11): at most once per `action_id`. `envelope.expect_generation` = project run count seen. */
  actStartRun: (envelope: Envelope, i: RunInput, cwd: string) => call<ActResult<RunHandle>>("act_start_run", { envelope, ...i, cwd }),
  /** Per worker of a run: whether it is closed, could be closed now, or why it is kept (with the exact counts of unsaved work). */
  closeoutStatus: (slug: string, runId: string) => call<{ workers: CloseoutRow[] }>("closeout_status", { slug, runId }),
  /** Close one finished worker. Without `confirmed` only a worker that loses nothing closes; "Close anyway" sends the counts the human saw (files, commits, busy terminals, no upstream), and anything that changed refuses. */
  actCloseWorker: (envelope: Envelope, worker: string, confirmed?: { changedFiles: number; unpushedCommits: number; busyTerminals: number; noUpstream: boolean }) => call<ActResult>("act_close_worker", { envelope, worker, changedFiles: confirmed?.changedFiles ?? null, unpushedCommits: confirmed?.unpushedCommits ?? null, busyTerminals: confirmed?.busyTerminals ?? null, noUpstream: confirmed?.noUpstream ?? null }),
  /** Close every finished worker that loses nothing; the result lists what was kept and why. */
  actCloseFinished: (envelope: Envelope) => call<ActResult<CloseFinishedResult>>("act_close_finished", { envelope }),
  workerDisk: () => call<WorkerDisk>("worker_disk"),
  /** Opens Apple's developer tools installer; the core refuses it once git is installed. */
  installDevTools: () => call<{ started: boolean }>("install_dev_tools"),
  actWorkerCleanUp: (envelope: Envelope) => call<ActResult<CleanUpResult>>("act_worker_clean_up", { envelope }),
  actStopWorker: (envelope: Envelope, worker: string, reason?: string) => call<ActResult>("act_stop_worker", { envelope, worker, reason: reason ?? null }),
  /** Reject plan `id` through a receipt. Pass the `plan_sha256` you read (REQUIRED: the core refuses a reject without it) as a STRING (never as `expect_generation`: a 64-bit hash prefix does not fit a JS number); a changed plan is refused as stale before anything runs. */
  actRejectPlan: (envelope: Envelope, id: string, reason: string, planSha256: string) => call<ActResult>("act_reject_plan", { envelope, id, reason, planSha256 }),
  /** Answer a waiting task: the text goes to its worker as a message (never a shell). Run-level questions are refused (`not_answerable`). */
  actAnswerQuestion: (envelope: Envelope, id: string, text: string) => call<ActResult>("act_answer_question", { envelope, id, text }),
  actApprovePlan: (envelope: Envelope, id: string, planSha256: string) => call<ActResult>("act_approve_plan", { envelope, id, planSha256 }),
  /** Pass the last `rev` as `sinceRev`; `unchanged` means nothing to redraw. */
  projectState: (projectId: string, sinceRev?: number) => call<ProjectState>("project_state", { projectId, sinceRev: sinceRev ?? null }),
  resumeRun: (sessionId: string, campaign: string, cwd: string, provider?: string) => call<RunHandle>("resume_run", { sessionId, campaign, cwd, ...(provider ? { provider } : {}) }),
  /** `vajra.providers/1`: each adapter with install state, version, capabilities (and `limits` reasons) and the CLI's own login check. Parse with `parseProviderList`. */
  providerList: () => call<ProviderList>("provider_list"),
  // Multi-provider accounts. Fixture-backed until the core commands land; answers are parsed with `parseAccounts`, `parseUsage` and `parseAgents`.
  providerAccounts: () => call<unknown>("provider_accounts"),
  providerUsage: () => call<unknown>("provider_usage"),
  /** Hands the pasted key to the core, which writes it to the Keychain. Nothing answers with it, and nothing here keeps it. */
  providerKeySaved: (accountId: string) => call<unknown>("provider_key_saved", { accountId }),
  providerKeySave: (accountId: string, key: string) => call<void>("provider_key_save", { accountId, key }),
  /** Overwrites the stored key. Save refuses a second key, so a saved account's Replace must come here. */
  providerKeyReplace: (accountId: string, key: string) => call<void>("provider_key_replace", { accountId, key }),
  providerKeyRemove: (accountId: string) => call<void>("provider_key_remove", { accountId }),
  providerBaseUrlSave: (accountId: string, baseUrl: string) => call<void>("provider_base_url_save", { accountId, baseUrl }),
  providerAgents: (projectId: string) => call<unknown>("provider_agents", { projectId }),
  providerAgentsSave: (projectId: string, agents: unknown) => call<unknown>("provider_agents_save", { projectId, agents }),
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
  // User terminals (core module `terminal`; never reachable by agents). Output arrives as raw bytes on the channel.
  termOpen: (cwd: string, rows: number, cols: number, onMsg: (m: TermMsg) => void) => termStream("term_open", { cwd, rows, cols }, onMsg),
  termAttach: (id: number, onMsg: (m: TermMsg) => void) => termStream("term_attach", { id }, onMsg),
  recoverOrphans: (pids: number[]) => call<{ results: { pid: number; result: string | { survived: { remaining: number } } }[] }>("recover_orphans", { pids }),
  diagnosticsPreview: () => call<{ text: string }>("diagnostics_preview"),
  lifecycle: () => call<Lifecycle>("lifecycle"),
  termList: () => call<{ terminals: TermInfo[] }>("term_list"),
  termWrite: (id: number, data: string) => call<Json>("term_write", { id, data }),
  termResize: (id: number, rows: number, cols: number) => call<Json>("term_resize", { id, rows, cols }),
  /** The pane is shown or hidden; `alt` says it was on the alternate screen when it hid. */
  termVisible: (id: number, visible: boolean, alt = false) => call<Json>("term_visible", { id, visible, alt }),
  termAck: (id: number, bytes: number) => call<Json>("term_ack", { id, bytes }),
  termClose: (id: number) => call<Json>("term_close", { id }),
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
  /** ADR-15 2: one headless turn of the run's project's Director-General per question. The reply streams into
   *  `stream` (`orchestrator-<project>-g<generation>`). */
  askLead: (runId: string, text: string) => call<AskStarted>("ask_lead", { runId, text }),
  cancelAsk: (askId: string) => call<Json>("cancel_ask", { askId }),
  leadStatus: (runId: string) => call<LeadStatus>("lead_status", { runId }),
  /** The project's Director-General with no run (FG-02c). The stream is rebuilt after a restart only by `leadStatus`. */
  leadStatusForProject: (projectId: string) => call<LeadStatus>("lead_status", { projectId }),
  /** Guarded Director-General actions (FG-02c, decision Q2): at most once per `action_id`. `envelope.project_id` is
   *  the run's project and `expect_generation` the conversation shown (`stale_generation` when it moved on). */
  actAskLead: (envelope: Envelope, runId: string, text: string) => call<ActResult<AskStarted>>("act_ask_lead", { envelope, runId, text }),
  actNewConversation: (envelope: Envelope, runId: string) => call<ActResult<NewConversation>>("act_new_conversation", { envelope, runId }),
  actCancelAsk: (envelope: Envelope, askId: string) => call<ActResult>("act_cancel_ask", { envelope, askId }),
  /** `envelope.project_id` is the project the inbox lists the proposal under. */
  actStartProposal: (envelope: Envelope, id: string, proposalSha256: string) => call<ActResult<{ action_id: string; state: string; run: Json }>>("act_start_proposal", { envelope, id, proposalSha256 }),
  actDismissProposal: (envelope: Envelope, id: string) => call<ActResult<{ action_id: string; state: string }>>("act_dismiss_proposal", { envelope, id }),
  /** The run's project's Director-General starts generation n+1, on an empty stream. Refused (`lead_busy`) mid-turn. */
  newConversation: (runId: string) => call<NewConversation>("new_conversation", { runId }),
  /** Everything waiting on the human, oldest first. Derived on each call; poll it like `crew`. */
  /** Approve plan `id` as shown: starts the approved action as a new turn. Refused (`plan_changed`) if the plan changed since `inbox` showed it. */
  approvePlan: (id: string, planSha256: string) => call<{ action_id: string; state: string; via: string }>("approve_plan", { id, planSha256 }),
  /** Reject plan `id` with a reason (1 to 500 characters). */
  rejectPlan: (id: string, reason: string) => call<{ action_id: string; state: string }>("reject_plan", { id, reason }),
  /** ADR-15 2.4: start a proposed run as shown. Refused (`proposal_changed`) unless `proposalSha256` is the shown hash. */
  startProposal: (id: string, proposalSha256: string, overrides: ProposalOverride[] = [], approveChecker = false) =>
    call<{ action_id: string; state: string; run: Json; seats?: StartReceiptSeat[]; evidence_seat?: string; evidence_trust?: string; campaign_slug?: string }>("start_proposal", { id, proposalSha256, ...(overrides.length > 0 ? { overrides } : {}), ...(approveChecker ? { approveChecker } : {}) }),
  dismissProposal: (id: string) => call<{ action_id: string; state: string }>("dismiss_proposal", { id }),
  inbox: () => call<{ count: number; campaign_count?: number; items: InboxItem[] }>("inbox"),
  /** Which record versions this build reads (`vajra.app_info/1`). */
  appInfo: () => call<AppInfo>("app_info"),
  /** Active projects, oldest first; `skipped` counts records that were damaged, foreign or from a newer version. */
  /** The project that owns a folder by the core's identity rules (symlink, linked worktree), else `p_unsorted`. Read only. */
  projectForFolder: (path: string) => call<{ project_id: string }>("project_for_folder", { path }),
  projectList: (includeArchived = false) => call<{ projects: Project[]; skipped: number; truncated: boolean }>("project_list", { includeArchived }),
  /** Add a folder. Fails `nested` (with `with: {id, name}`) when it overlaps a project unless `confirmNested`; `refused_folder` for home, the Claude folder or Vajra's data. */
  projectAdd: (path: string, confirmNested = false) =>
    call<{ outcome: "created" | "existing" | "restored"; project: Project }>("project_add", { path, confirmNested }),
  /** Fails `conflict` (with `current_rev`) when `expectRev` is stale. */
  projectRename: (id: string, name: string, expectRev?: number) => call<{ project: Project }>("project_rename", { id, name, expectRev }),
  projectArchive: (id: string, expectRev?: number) => call<{ project: Project }>("project_archive", { id, expectRev }),
  /** Suggested projects for campaigns from before projects existed. Writes nothing; `done` once confirmed. `terminal` campaigns are read-only and never adopted. */
  projectMigrationSuggest: () =>
    call<{ done: boolean; groups: { root: string; name: string; campaigns: string[] }[]; unsorted: string[]; terminal: string[] }>("project_migration_suggest"),
  /** Confirm the accepted suggested roots, once. Fails `already_migrated` the second time. */
  projectMigrationConfirm: (roots: string[]) =>
    call<{ projects: { id: string; name: string; outcome: string; sessions: number }[]; nested: string[]; unknown: string[] }>("project_migration_confirm", { roots }),
  /** `skipped` counts unusable lines; `unsupported` means a newer build wrote records this one cannot read, so show "update needed"; `truncated` means the log is longer than the window read, so older sessions may be missing. */
  projectSessions: (id: string) => call<{ sessions: Session[]; skipped: number; unsupported: boolean; truncated: boolean }>("project_sessions", { id }),
};

/** The core says something changed under the Vajra state dir; payload is the list of changed campaign slugs. */
/** `inbox_new`: something new started waiting; carries ids, a kind and a count, never item text. Coalesced over 5 s in the core. */
export type InboxNew = { project_id: string; kind: string; count: number };
export const onInboxNew = (cb: (payload: InboxNew) => void) => transport.on("inbox_new", (p) => cb(p as InboxNew));
export const onChanged = (cb: (payload: unknown) => void) => transport.on("changed", cb);
