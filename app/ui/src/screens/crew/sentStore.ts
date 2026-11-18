// The messages the human sent to each worker from this window, so their delivery state survives switching panes. Held in
// memory only and capped: the worker's own transcript is the record (a message that reached it is in there).
import { useSyncExternalStore } from "react";
import type { Sent } from "./chat";

/** Messages kept per worker, and workers kept; the oldest go first. */
export const SENT_PER_WORKER = 20;
export const SENT_WORKERS = 50;

const byWorker = new Map<string, readonly Sent[]>();
const subs = new Set<() => void>();
const EMPTY: readonly Sent[] = [];

function set(worker: string, list: readonly Sent[]) {
  byWorker.delete(worker);
  byWorker.set(worker, list.slice(-SENT_PER_WORKER));
  while (byWorker.size > SENT_WORKERS) byWorker.delete(byWorker.keys().next().value as string);
  subs.forEach((f) => f());
}

export const sentStore = {
  get: (worker: string): readonly Sent[] => byWorker.get(worker) ?? EMPTY,
  add: (worker: string, s: Sent) => set(worker, [...sentStore.get(worker), s]),
  update: (worker: string, key: string, patch: Partial<Sent>) =>
    set(worker, sentStore.get(worker).map((s) => (s.key === key ? { ...s, ...patch } : s))),
  /** These messages reached the worker (their block is in its transcript): Seen from now on, in any window. */
  markSeen: (worker: string, ids: readonly string[]) => {
    const list = sentStore.get(worker);
    if (!list.some((s) => s.id && ids.includes(s.id) && s.state !== "seen")) return;
    set(worker, list.map((s) => (s.id && ids.includes(s.id) ? { ...s, state: "seen" as const } : s)));
  },
  /** Tests only. */
  clear: () => { byWorker.clear(); subs.forEach((f) => f()); },
  subscribe: (f: () => void) => { subs.add(f); return () => { subs.delete(f); }; },
};

export function useSent(worker: string): readonly Sent[] {
  return useSyncExternalStore(sentStore.subscribe, () => sentStore.get(worker), () => EMPTY);
}
