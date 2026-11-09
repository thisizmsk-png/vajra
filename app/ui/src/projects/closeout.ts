// What closing a run's finished workers would do, per worker (the core decides; this only reads and shows it).
import { useCallback, useEffect, useRef, useState } from "react";
import { api, onChanged } from "../ipc";
import type { CloseoutRow } from "../ipc";
import { S } from "../../strings";

const MIN_GAP_MS = 2000;

/** The run's workers keyed by id. A failed read keeps what was shown: no row is invented, nothing is claimed closed. */
export function useCloseout(slug: string | null, runId: string | null) {
  const [rows, setRows] = useState<Map<string, CloseoutRow>>(new Map());
  const seq = useRef(0);
  const last = useRef(0);
  const load = useCallback(() => {
    if (!slug || !runId) return;
    const mine = ++seq.current;
    last.current = Date.now();
    api.closeoutStatus(slug, runId).then(
      (r) => { if (mine === seq.current && Array.isArray(r?.workers)) setRows(new Map(r.workers.map((w) => [w.worker, w]))); },
      () => {},
    );
  }, [slug, runId]);
  useEffect(() => { setRows(new Map()); load(); return () => { seq.current++; }; }, [load]);
  useEffect(() => {
    if (!slug || !runId) return;
    let later: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged(() => {
      const wait = MIN_GAP_MS - (Date.now() - last.current);
      if (wait <= 0) load();
      else if (later === undefined) later = setTimeout(() => { later = undefined; load(); }, wait);
    });
    return () => { off(); clearTimeout(later); };
  }, [slug, runId, load]);
  return { rows, reload: load };
}

/** "3 changed files, 2 unpushed commits" — the exact counts; "no upstream branch" when nothing tracks the branch. */
export function unsavedText(r: { changed_files?: number; unpushed_commits?: number; no_upstream?: boolean }): string {
  const parts = [S.closeout.files(r.changed_files ?? 0), S.closeout.commits(r.unpushed_commits ?? 0)];
  if (r.no_upstream) parts.push(S.closeout.noUpstream);
  return parts.join(", ");
}
