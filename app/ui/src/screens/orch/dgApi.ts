// Everything the Director-General view reads or does, in one place. The interface is FG-02c's: the Director-General
// is addressed by its project (the core's T1 project id), every action carries the FG-11 envelope (an action id made
// once per click and reused on retry, the project, and the generation the UI showed), and every action answers with
// an outcome in the receipt's words (ADR-15 §3.2).
//
// INTERIM until FG-02c lands (vajra-13, after #82/#83): the core still keys these commands by a run of the project,
// and start/dismiss take no envelope, so the implementation below calls #83's run-keyed commands and ignores the
// envelope. FG-02c's replacements, one for one:
//   status                -> lead_status { projectId }                (a read, no envelope)
//   ask                   -> act_ask_lead { envelope, text }            expect_generation = the conversation shown
//   newConversation       -> act_new_conversation { envelope }          expect_generation = the conversation shown
//   cancel                -> act_cancel_ask { envelope, askId }         expect_generation null
//   startProposal         -> act_start_proposal { envelope, id, proposalSha256 }  expect_generation = project_state.generations.project
//   dismissProposal       -> act_dismiss_proposal { envelope, id }      expect_generation null
// Reloads follow useDgReload, a thin wrapper over the shared useProjectState once it lands (vajra-11).
import { useEffect, useRef } from "react";
import { S } from "../../../strings";
import { api, errorText, isIpcError, onChanged } from "../../ipc";
import type { AskStarted, InboxItem, LeadStatus, NewConversation } from "../../ipc";
import { normRoot } from "../../projects/adapter";

/** Who the commands are for. `projectId` is the core's id (from its own answers); `runId` serves the interim calls. */
export type Target = { projectId: string | null; runId: string | null };
/** FG-11's envelope (ADR-15 §3.1). */
export type Envelope = { action_id: string; project_id: string | null; expect_generation: number | null };
export type Outcome = { kind: "pending" | "done" | "refused" | "unknown"; text: string };
export type Result<T> = { ok: true; value: T } | { ok: false; outcome: Outcome; code: string | null };

/** `provider_error` (H9): a worker's failure, typed by the core, which also owns the retry policy. The message arrives
 *  already redacted. Nothing here switches an account or a provider; moving a worker is a new pick the user confirms. */
export type ProviderErrorKind = "auth" | "quota" | "rate_limit" | "timeout" | "upstream" | "bad_request" | "unexplained_exit";
export type ProviderErrorEvent = {
  type: "provider_error";
  slot: string;
  session_id: string;
  account: string;
  kind: ProviderErrorKind;
  http_status: number | null;
  retry_after_s: number | null;
  attempt: number;
  message: string;
  /** `retrying`: the core runs the same worker again on the same account. `asking_user`: the task is blocked. */
  action: "retrying" | "asking_user";
};
const PROVIDER_ERROR_KINDS: readonly string[] = ["auth", "quota", "rate_limit", "timeout", "upstream", "bad_request", "unexplained_exit"];
/** Narrow an unknown event; anything with an unknown kind or action is not one. */
export function isProviderError(v: unknown): v is ProviderErrorEvent {
  const e = v as Partial<ProviderErrorEvent> | null;
  return !!e && e.type === "provider_error" && typeof e.slot === "string" && typeof e.session_id === "string" && typeof e.account === "string" &&
    typeof e.kind === "string" && PROVIDER_ERROR_KINDS.includes(e.kind) && (e.action === "retrying" || e.action === "asking_user") &&
    typeof e.attempt === "number" && typeof e.message === "string";
}

/** A fresh action id for one click. A retry after an unknown outcome reuses it, so the core can answer it once. */
export function actionId(): string {
  const b = new Uint8Array(12);
  crypto.getRandomValues(b);
  return `a_${Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("")}`;
}
export const envelope = (t: Target, id: string, expect: number | null): Envelope => ({ action_id: id, project_id: t.projectId, expect_generation: expect });

/** A failure as the receipt would say it: a core refusal with its reason, or unknown when no answer came back. */
export function failure(e: unknown, changed = S.dg.receipt.changed): { outcome: Outcome; code: string | null } {
  if (!isIpcError(e)) return { outcome: { kind: "unknown", text: S.dg.receipt.unknown }, code: null };
  if (e.error === "proposal_changed" || e.error === "stale_generation") return { outcome: { kind: "refused", text: changed }, code: e.error };
  if (e.error === "lead_busy") return { outcome: { kind: "refused", text: S.dg.busy }, code: e.error };
  return { outcome: { kind: "refused", text: S.dg.receipt.refused(errorText(e)) }, code: e.error };
}

async function run<T>(f: () => Promise<T>): Promise<Result<T>> {
  try { return { ok: true, value: await f() }; } catch (e) { return { ok: false, ...failure(e) }; }
}
const noRun = () => Promise.reject({ error: "no_run", detail: S.dg.noRun });

export const dg = {
  /** The tail of a conversation's stream (a read; not keyed by run, unchanged by FG-02c). */
  window: (stream: string, rows: number) => api.streamWindow(stream, null, rows),
  status: (t: Target): Promise<LeadStatus> => (t.runId ? api.leadStatus(t.runId) : noRun()),
  ask: (t: Target, env: Envelope, text: string): Promise<Result<AskStarted>> => run(() => (t.runId ? api.askLead(t.runId, text) : noRun())),
  newConversation: (t: Target, env: Envelope): Promise<Result<NewConversation>> => run(() => (t.runId ? api.newConversation(t.runId) : noRun())),
  cancel: (t: Target, env: Envelope, askId: string): Promise<Result<unknown>> => run(() => api.cancelAsk(askId)),
  startProposal: async (t: Target, env: Envelope, item: InboxItem): Promise<Outcome> => {
    if (!item.proposal_sha256) return { kind: "refused", text: S.dg.receipt.changed };
    const r = await run(() => api.startProposal(item.id, item.proposal_sha256!));
    return r.ok ? { kind: "done", text: S.dg.receipt.started } : r.outcome;
  },
  dismissProposal: async (t: Target, env: Envelope, item: InboxItem): Promise<Outcome> => {
    const r = await run(() => api.dismissProposal(item.id));
    return r.ok ? { kind: "done", text: S.dg.receipt.dismissed } : r.outcome;
  },
};

/**
 * This project's run proposals: by the core's T1 id for a registered project (it survives a rename or a move), and by
 * the full root the proposal names (FG-02b sends the recorded root) for any project.
 */
export function proposalsFor(inbox: InboxItem[], projectId: string | null, root: string | null): InboxItem[] {
  return inbox.filter((i) => i.kind === "run_proposal" && (
    (!!projectId && i.project_id === projectId)
    || (!!root && typeof i.project === "string" && normRoot(i.project) === normRoot(root))));
}

/**
 * Call `reload` when this Director-General's state may have moved. Interim: a `changed` event naming its stream, at
 * most once per `gapMs`. Becomes useProjectState(projectId).rev as the key (first value is the baseline), with this
 * as the fallback while that store reports `failed`.
 */
export function useDgReload(stream: string | null, reload: () => void, gapMs = 500) {
  const fn = useRef(reload);
  fn.current = reload;
  useEffect(() => {
    if (!stream) return;
    let last = 0;
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged((payload) => {
      const p = payload as { streams?: unknown } | null;
      if (!Array.isArray(p?.streams) || !p.streams.includes(stream)) return;
      const wait = gapMs - (Date.now() - last);
      const go = () => { last = Date.now(); fn.current(); };
      if (wait <= 0) go();
      else if (later === undefined) later = setTimeout(() => { later = undefined; go(); }, wait);
    });
    return () => { off(); clearTimeout(later); };
  }, [stream, gapMs]);
}
