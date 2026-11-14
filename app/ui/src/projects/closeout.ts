// What closing a run's finished workers would do, per worker (the core decides; this only reads and shows it).
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../ipc";
import { onChangedPaced } from "./poll";
import type { CloseoutRow } from "../ipc";
import { S } from "../../strings";

const MIN_GAP_MS = 2000;

/** The run's workers keyed by id. A failed read keeps what was shown: no row is invented, nothing is claimed closed. */
export function useCloseout(slug: string | null, runId: string | null) {
  const [rows, setRows] = useState<Map<string, CloseoutRow>>(new Map());
  const seq = useRef(0);
  const load = useCallback(() => {
    if (!slug || !runId) return;
    const mine = ++seq.current;
    api.closeoutStatus(slug, runId).then(
      (r) => { if (mine === seq.current && Array.isArray(r?.workers)) setRows(new Map(r.workers.map((w) => [w.worker, w]))); },
      () => {},
    );
  }, [slug, runId]);
  useEffect(() => { setRows(new Map()); load(); return () => { seq.current++; }; }, [load]);
  // Paced; hidden: held back, one read on showing.
  useEffect(() => (slug && runId ? onChangedPaced(MIN_GAP_MS, load) : undefined), [slug, runId, load]);
  return { rows, reload: load };
}

/** "3 changed files, 2 unpushed commits, 1 terminal still running something": the exact counts; "no upstream branch" when nothing tracks the branch. */
export function unsavedText(r: { changed_files?: number; unpushed_commits?: number; no_upstream?: boolean; busy_terminals?: number }): string {
  const parts: string[] = [];
  // A worker kept only for a busy terminal has clean git: leave the zero counts out rather than list them.
  if (r.busy_terminals === undefined || r.busy_terminals === 0 || r.changed_files || r.unpushed_commits || r.no_upstream) parts.push(S.closeout.files(r.changed_files ?? 0), S.closeout.commits(r.unpushed_commits ?? 0));
  if (r.no_upstream) parts.push(S.closeout.noUpstream);
  if (r.busy_terminals) parts.push(S.closeout.busy(r.busy_terminals));
  return parts.join(", ");
}
