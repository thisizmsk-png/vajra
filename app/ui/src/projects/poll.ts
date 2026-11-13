// Nothing runs at rest. A poll is for data that changes without an event (a live run's crew, tasks and stream), so it
// exists only while `on` (its run is live) and the window is visible. Hidden: no timer at all, not a timer that skips.
// Shown again: one read at once, then the cadence. Events (`changed`) are separate and still reach the screen.
import { useEffect, useRef } from "react";
import { onChangedRaw } from "../ipc";

/**
 * Read on `changed` events, at most once per `gapMs` (leading and trailing: an event inside the gap is read when it
 * ends). While the window is hidden nothing is read: events and a pending trailing read only mark the view stale, and
 * showing the window reads once. `accept` drops events that are not about this view.
 */
export function onChangedPaced(gapMs: number, fn: () => void, accept: (payload: unknown) => boolean = () => true): () => void {
  let last = 0;
  let later: ReturnType<typeof setTimeout> | undefined;
  let stale = false;
  const go = () => {
    if (document.hidden) { stale = true; return; }
    last = Date.now();
    fn();
  };
  const off = onChangedRaw((p) => {
    if (!accept(p)) return;
    if (document.hidden) { stale = true; return; }
    const wait = gapMs - (Date.now() - last);
    if (wait <= 0) go();
    else if (later === undefined) later = setTimeout(() => { later = undefined; go(); }, wait);
  });
  const onVisibility = () => {
    if (document.hidden) {
      if (later !== undefined) { clearTimeout(later); later = undefined; stale = true; }
    } else if (stale) {
      stale = false;
      go();
    }
  };
  document.addEventListener("visibilitychange", onVisibility);
  return () => { clearTimeout(later); off(); document.removeEventListener("visibilitychange", onVisibility); };
}

export function usePoll(fn: () => void, ms: number, on: boolean): void {
  const latest = useRef(fn);
  latest.current = fn;
  useEffect(() => {
    if (!on) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const start = () => {
      if (timer === undefined) timer = setInterval(() => latest.current(), ms);
    };
    const stop = () => {
      clearInterval(timer);
      timer = undefined;
    };
    const onVisibility = () => {
      if (document.hidden) stop();
      else {
        latest.current();
        start();
      }
    };
    if (!document.hidden) start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [ms, on]);
}
