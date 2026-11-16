import { useEffect, useState } from "react";
import { api } from "../ipc";

/**
 * True only when the core says the folder exists and is not a git repository: a run has no HEAD to start from there, so the
 * launcher must not start. A path that is not absolute yet, a check that fails (an older core without the command) and a
 * check still in flight all read false, so a typing pause or a missing answer never blocks a real repository.
 */
export function useNotGit(path: string, delayMs = 300): boolean {
  const [state, setState] = useState<{ path: string; notGit: boolean } | null>(null);
  const trimmed = path.trim();
  useEffect(() => {
    if (!trimmed.startsWith("/")) return;
    let live = true;
    const t = setTimeout(() => {
      api.folderIsGit(trimmed).then(
        (r) => { if (live) setState({ path: trimmed, notGit: r?.git === false }); },
        () => { if (live) setState({ path: trimmed, notGit: false }); },
      );
    }, delayMs);
    return () => { live = false; clearTimeout(t); };
  }, [trimmed, delayMs]);
  // An answer belongs to the path it was asked for: an edited field does not keep the last folder's verdict.
  return state !== null && state.path === trimmed && state.notGit;
}
