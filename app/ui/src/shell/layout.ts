// How the window is laid out, not where you are: the hash says where, this says how. Kept in localStorage until the
// core's settings store (FG-08) can hold it. Each value is a choice you made; with no choice the window size decides.
import { useCallback, useState } from "react";

const KEY = { compact: "vajra.layout.compact", inspector: "vajra.layout.inspector", project: "vajra.project" } as const;

/** Below this width the project column starts folded: at 960 the column would leave the main area 688 px. */
export const COMPACT_BELOW = 1100;
/** Below this width the inspector starts closed, so the main area stays the largest region (shell B rule). */
export const INSPECTOR_FROM = 1440;

function read(key: string): string | null {
  try { return window.localStorage.getItem(key); } catch { return null; }
}
function write(key: string, v: string | null) {
  try { if (v === null) window.localStorage.removeItem(key); else window.localStorage.setItem(key, v); } catch { /* private mode: the choice lasts the session */ }
}
const width = () => (typeof window === "undefined" ? 1440 : window.innerWidth);

function useFlag(key: string, fallback: () => boolean): [boolean, (v: boolean | ((x: boolean) => boolean)) => void] {
  const [v, setV] = useState(() => { const s = read(key); return s === "1" ? true : s === "0" ? false : fallback(); });
  const set = useCallback((next: boolean | ((x: boolean) => boolean)) => {
    setV((x) => { const n = typeof next === "function" ? next(x) : next; write(key, n ? "1" : "0"); return n; });
  }, [key]);
  return [v, set];
}

export const useCompact = () => useFlag(KEY.compact, () => width() < COMPACT_BELOW);
export const useInspector = () => useFlag(KEY.inspector, () => width() >= INSPECTOR_FROM);
export const lastProject = () => read(KEY.project);
export const rememberProject = (id: string | null) => write(KEY.project, id);
