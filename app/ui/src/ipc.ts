// The one place that names the core's IPC commands. Screens call `api.*`, never `invoke`, so a screen can be tested
// against `setTransport(fake)` and a command rename is a one-file change. Argument names are camelCase here; Tauri maps
// them to the snake_case parameters of the Rust commands.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** What a failed command rejects with: `{error: code, detail?}` from the core (a control error adds its own fields). */
export type IpcError = { error: string; detail?: string; [k: string]: unknown };
export const isIpcError = (e: unknown): e is IpcError => typeof e === "object" && e !== null && typeof (e as IpcError).error === "string";

/** A short line safe to show: the code and detail from the core, or a plain fallback. Never a stack. */
export function errorText(e: unknown): string {
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
};
export type RunHandle = { stream: string; session_id: string; run_id: string; campaign: string };
export type CrewNodeDto = { id: string; kind: string; title: string; engine_id: string | null; status: string; overlays: unknown[]; children: CrewNodeDto[] };
export type Line = { seq: number; text: string };
export type Tail = { lines: Line[]; gap: boolean; head_seq: number; spill_start: number | null; spill_end: number | null };
export type ResumeTarget = { session_id: string; run_id: string; cwd: string };
export type ActiveRuns = { count: number; runs: unknown[] };
export type Json = Record<string, unknown>;
export type RunInput = { ask: string; maxRounds: number; base: string; maxInFlight: number; campaign: string | null };

export const api = {
  preflight: () => call<Preflight>("preflight"),
  listCampaigns: (refresh = false) => call<{ campaigns: Campaign[] }>("list_campaigns", { refresh }),
  startRun: (i: RunInput, cwd: string) => call<RunHandle>("start_run", { ...i, cwd }),
  resumeRun: (sessionId: string, campaign: string, cwd: string) => call<RunHandle>("resume_run", { sessionId, campaign, cwd }),
  crew: (slug: string, runId: string) => call<CrewNodeDto>("crew", { slug, runId }),
  tail: (stream: string, afterSeq: number, maxBytes = 65536) => call<Tail>("tail", { stream, afterSeq, maxBytes }),
  readSpill: (stream: string, offset: number, maxBytes = 65536) => call<Json>("read_spill", { stream, offset, maxBytes }),
  task: (slug: string, taskId: string) => call<Json>("task", { slug, taskId }),
  diff: (cwd: string, base: string) => call<{ code: number | null; diff: string; truncated: boolean; timed_out: boolean }>("diff", { cwd, base }),
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
};

/** The core says something changed under the Vajra state dir; payload is the list of changed campaign slugs. */
export const onChanged = (cb: (payload: unknown) => void) => transport.on("changed", cb);
