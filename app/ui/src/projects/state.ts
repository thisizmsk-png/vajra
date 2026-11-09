// One revisioned reader of a project's state (ADR-15 §4): every surface that wants to know "did this project move?"
// shares one `changed` subscription and one `project_state` fetch per 500 ms gap, and only hears about a newer revision.
import { useEffect, useState } from "react";
import { api, onChanged } from "../ipc";
import type { Changed } from "../ipc";

export const STATE_MIN_GAP_MS = 500;

export type ProjectStateView = { rev: number | null; failed: boolean };

type Entry = {
  view: ProjectStateView;
  users: Set<(v: ProjectStateView) => void>;
  off: () => void;
  timer: ReturnType<typeof setTimeout> | undefined;
  last: number;
  busy: boolean;
  again: boolean;
  dead: boolean;
};

const entries = new Map<string, Entry>();

function publish(e: Entry, view: ProjectStateView) {
  if (view.rev === e.view.rev && view.failed === e.view.failed) return;
  e.view = view;
  e.users.forEach((u) => u(view));
}

function refresh(id: string, e: Entry) {
  if (e.dead) return;
  if (e.busy) { e.again = true; return; }
  e.busy = true;
  e.last = Date.now();
  void api.projectState(id, e.view.rev ?? undefined).then(
    (s) => {
      if (e.dead) return;
      // `unchanged` repeats our revision, so publish() sees no difference and nobody re-renders.
      publish(e, { rev: s.rev, failed: false });
    },
    () => { if (!e.dead) publish(e, { rev: e.view.rev, failed: true }); },
  ).finally(() => {
    e.busy = false;
    if (e.again && !e.dead) { e.again = false; schedule(id, e); }
  });
}

function schedule(id: string, e: Entry) {
  if (e.timer !== undefined || e.dead) return;
  const wait = Math.max(0, STATE_MIN_GAP_MS - (Date.now() - e.last));
  e.timer = setTimeout(() => { e.timer = undefined; refresh(id, e); }, wait);
}

function open(id: string): Entry {
  const e: Entry = { view: { rev: null, failed: false }, users: new Set(), off: () => {}, timer: undefined, last: 0, busy: false, again: false, dead: false };
  e.off = onChanged((payload) => {
    const mine = (payload as Changed | null)?.projects?.find((p) => p.project_id === id);
    // An event that names our project with a revision we already hold is old news. One without a list: look once.
    if (mine && e.view.rev !== null && mine.rev <= e.view.rev) return;
    if ((payload as Changed | null)?.projects && !mine) return;
    schedule(id, e);
  });
  refresh(id, e);
  return e;
}

/** The project's revision, or `failed` when the core could not answer (the caller then falls back to raw events). */
export function useProjectState(projectId: string | null): ProjectStateView {
  const [view, setView] = useState<ProjectStateView>({ rev: null, failed: false });
  useEffect(() => {
    if (!projectId) { setView({ rev: null, failed: false }); return; }
    let e = entries.get(projectId);
    if (!e) { e = open(projectId); entries.set(projectId, e); }
    const entry = e;
    entry.users.add(setView);
    setView(entry.view);
    return () => {
      entry.users.delete(setView);
      if (entry.users.size === 0) {
        entry.dead = true;
        entry.off();
        clearTimeout(entry.timer);
        if (entries.get(projectId) === entry) entries.delete(projectId);
      }
    };
  }, [projectId]);
  return projectId ? view : { rev: null, failed: false };
}
