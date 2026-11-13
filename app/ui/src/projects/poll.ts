// Nothing runs at rest. A poll is for data that changes without an event (a live run's crew, tasks and stream), so it
// exists only while `on` (its run is live) and the window is visible. Hidden: no timer at all, not a timer that skips.
// Shown again: one read at once, then the cadence. Events (`changed`) are separate and still reach the screen.
import { useEffect, useRef } from "react";

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
