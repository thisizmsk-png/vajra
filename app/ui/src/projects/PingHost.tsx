// The one place pings happen. It follows `inbox_new` (the core's coalesced "something new is waiting"), applies the
// ping rule (ping.ts) and shows each ping as a toast that says why it pinged. Arrivals the rule keeps quiet are logged
// with their reason, so the Needs-you queue can show what pinged and what did not. At idle it holds no timer: one
// timeout exists only while a ping is due.
import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { ToastRegion, useToasts } from "../../components/ui/base";
import { S } from "../../strings";
import { onInboxNew } from "../ipc";
import type { InboxNew } from "../ipc";
import { usePreferences } from "../preferences";
import { DEFAULT_PING } from "../preferenceContract";
import { EMPTY_PING_STATE, arrive, flush } from "./ping";
import type { PingEntry, PingState } from "./ping";
import { useProjects } from "./store";

/** How many log lines the queue can show; older ones drop. Session only: nothing about pings is stored. */
const LOG_MAX = 20;

const Log = createContext<PingEntry[]>([]);
/** The pings (and quiet arrivals) since the app opened, newest first. */
export const usePingLog = () => useContext(Log);

/** Read the clock at call time (never a captured `Date.now`), and keep one reference so the subscription stays put. */
const clock = () => Date.now();

export function PingHost({ children, now = clock }: { children: ReactNode; now?: () => number }) {
  const prefs = usePreferences();
  const P = useProjects();
  const { toasts, push, dismiss } = useToasts();
  const [log, setLog] = useState<PingEntry[]>([]);
  const state = useRef<PingState>(EMPTY_PING_STATE);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Read through refs so the subscription is made once and never misses an event while the rule or names change.
  const rule = useRef(DEFAULT_PING);
  // The rule is global (the core refuses a project one); notifications may be set per project.
  rule.current = prefs.view?.global.ping ?? DEFAULT_PING;
  const notify = useRef(true);
  notify.current = prefs.view?.effective.notifications ?? true;
  const names = useRef(P.projects);
  names.current = P.projects;
  const name = useCallback((id: string) => names.current.find((p) => p.id === id)?.name ?? S.projects.unsorted, []);

  const record = useCallback((e: PingEntry) => setLog((l) => [e, ...l].slice(0, LOG_MAX)), []);
  const arm = useCallback(() => {
    clearTimeout(timer.current);
    timer.current = undefined;
    const due = state.current.due;
    if (due === null) return;
    timer.current = setTimeout(() => {
      timer.current = undefined;
      const r = flush(state.current, now(), rule.current, name);
      state.current = r.state;
      if (r.ping) {
        record(r.ping);
        push("info", r.ping.text, r.ping.why, S.inbox.pinged);
      }
      arm();
    }, Math.max(0, due - now()));
  }, [name, now, push, record]);

  useEffect(() => {
    const off = onInboxNew((a: InboxNew) => {
      if (!a || typeof a.project_id !== "string" || typeof a.kind !== "string") return;
      const r = arrive(state.current, a, now(), rule.current, notify.current, name);
      state.current = r.state;
      if (r.quiet) record(r.quiet);
      arm();
    });
    return () => { off(); clearTimeout(timer.current); };
  }, [arm, name, now, record]);

  return (
    <Log.Provider value={log}>
      {children}
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </Log.Provider>
  );
}
