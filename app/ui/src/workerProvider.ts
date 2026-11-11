// What a worker (a crew node, a project_state worker, a Board card) says about the provider it RAN on: the `provider` object
// the core derives from the launch receipt's `effective` side (spec section 4). `requested` is present only when the CLI
// ran something other than what was asked for ("Ran on X (asked for Y)"). Nothing here is a credential. Kept small and
// dependency-free: it is in the idle shell.
import { S } from "../strings";
import { CONTROL, INVISIBLE } from "./projectContract";
import type { BillingKind } from "./accountsContract";

export type ProviderErrorKind = "timeout" | "upstream" | "rate_limit" | "auth" | "quota" | "unexplained_exit";
export type ProviderErrorAction = "retrying" | "asking_user" | "waiting";
export interface ProviderError { kind: ProviderErrorKind; action: ProviderErrorAction; attempt: number | null; retry_at: string | null }
export interface WorkerProvider {
  account: string;
  /** The provider's name as the core knows it ("Claude Code", "OpenRouter"): shown beside every model. */
  label: string;
  /** The model family (`claude`, `grok`, `deepseek`) that picks the icon; the account's own provider when there is none. */
  family: string;
  /** The model id actually running; null is the CLI's default model, "unknown" is "the harness did not report it". */
  model: string | null;
  model_name: string | null;
  billing: BillingKind;
  requested: { model: string | null; model_name: string | null; label: string } | null;
  error: ProviderError | null;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const text = (v: unknown, max = 128): string | null => (typeof v === "string" && v.trim() !== "" && [...v].length <= max && !CONTROL.test(v) && !INVISIBLE.test(v) ? v : null);
const KINDS: readonly ProviderErrorKind[] = ["timeout", "upstream", "rate_limit", "auth", "quota", "unexplained_exit"];
const ACTIONS: readonly ProviderErrorAction[] = ["retrying", "asking_user", "waiting"];
const BILLING: readonly BillingKind[] = ["subscription", "api_key", "gateway", "unknown"];

/** Absent or unreadable reads as "no provider to show": a surface then draws what it drew before this feature, never a guess. */
export function parseWorkerProvider(v: unknown): WorkerProvider | null {
  if (!isObj(v)) return null;
  const account = text(v.account, 64), label = text(v.label), family = text(v.family, 64);
  if (!account || !label || !family) return null;
  const e = v.error;
  const err = isObj(e) && KINDS.includes(e.kind as ProviderErrorKind)
    ? { kind: e.kind as ProviderErrorKind, action: ACTIONS.find((a) => a === e.action) ?? "asking_user", attempt: typeof e.attempt === "number" ? e.attempt : null, retry_at: text(e.retry_at, 32) }
    : null;
  const req = isObj(v.requested) ? { model: text(v.requested.model), model_name: text(v.requested.model_name), label: text(v.requested.label) ?? label } : null;
  return {
    account, label, family, model: v.model === null || v.model === undefined ? null : text(v.model), model_name: text(v.model_name),
    billing: BILLING.find((b) => b === v.billing) ?? "unknown", requested: req, error: err,
  };
}

/** A worker's change set (`diff` on a crew node or a `project_state` worker): the counts and the first files. */
export interface WorkerDiff { files_total: number; files: string[]; added: number; removed: number; partial: boolean; uncounted: number }
const count = (v: unknown): number | null => (typeof v === "number" && Number.isSafeInteger(v) && v >= 0 ? v : null);

/** Absent or malformed reads as "no diff to show". Paths are shown as text only, so a control or invisible character is dropped. */
export function parseWorkerDiff(v: unknown): WorkerDiff | null {
  if (!isObj(v)) return null;
  const files_total = count(v.files_total), added = count(v.added), removed = count(v.removed), uncounted = count(v.uncounted ?? 0);
  if (files_total === null || added === null || removed === null || uncounted === null || typeof v.partial !== "boolean") return null;
  const files = (Array.isArray(v.files) ? v.files : []).slice(0, 200).filter((f): f is string => typeof f === "string" && f.length <= 4096 && !CONTROL.test(f) && !INVISIBLE.test(f));
  return { files_total, files, added, removed, partial: v.partial, uncounted };
}

const W = S.workerProvider;
/** `+N −M` when the count is exact; `≥ +N lines` when it is only a lower bound. */
export const diffText = (d: WorkerDiff): string => (d.partial ? W.diff.atLeast(d.added) : W.diff.exact(d.added, d.removed));
/** The accessible label: what the short form means. */
export const diffLabel = (d: WorkerDiff): string => (d.partial ? W.diff.atLeastLabel(d.added, d.uncounted) : W.diff.exactLabel(d.added, d.removed, d.files_total));
/** "Model · provider": the model's name (or id, or the CLI default, or "Not reported") with its provider. Never a bare model. */
export const providerText = (label: string, model: string | null, name: string | null): string =>
  `${model === "unknown" ? W.notReported : name ?? model ?? S.accounts.defaultModel} · ${label}`;
export const workerLine = (p: WorkerProvider): string => providerText(p.label, p.model, p.model_name);
/** "Ran on X (asked for Y)" when the harness ran something other than what was asked for; null when it matches or nothing was asked. */
export function ranOn(p: WorkerProvider): string | null {
  const r = p.requested;
  if (!r || (r.model === p.model && r.model_name === p.model_name && r.label === p.label)) return null;
  return W.ranOn(workerLine(p), providerText(r.label, r.model, r.model_name));
}
export function errorText(e: ProviderError): string {
  const at = e.retry_at;
  if (e.action === "retrying") return W.error.retrying[e.kind](e.attempt, at);
  return W.error.needsYou[e.kind](at);
}
