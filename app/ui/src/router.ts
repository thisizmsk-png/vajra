// A hash router with no dependency: `#/p/<project>/<view>?a=b`. A hash needs no server and survives a reload in the
// webview. Views belong to a project; `#/<view>` (no project) is also accepted, so old links and screens that only know
// a campaign keep working, and the shell picks the project that owns the campaign.
import { useCallback, useSyncExternalStore } from "react";

/** The views of shell B. `crew` and `history` are the full-tree and all-campaign screens, kept until their slices replace them. */
export const SCREENS = ["orch", "launch", "board", "review", "worker", "crew", "history", "inbox", "settings"] as const;
export type Screen = (typeof SCREENS)[number];
/** Names from the screen-list shell, still used by links and by screens that predate projects. */
const ALIAS = { home: "orch", run: "review", chat: "orch" } as const;
export type AnyScreen = Screen | keyof typeof ALIAS;
export type Route = { screen: Screen; params: Record<string, string>; project: string | null };

const isScreen = (s: string): s is Screen => (SCREENS as readonly string[]).includes(s);
const canon = (s: string): Screen | null => (isScreen(s) ? s : Object.prototype.hasOwnProperty.call(ALIAS, s) ? ALIAS[s as keyof typeof ALIAS] : null);
/** A project id is a plain token (the adapter's `p_<hex>`, the core's ULIDs): anything else is not a project. */
const PROJECT = /^[A-Za-z0-9_-]{1,64}$/;

/** Anything that is not a known view is the orchestrator view: a stale or hand-typed hash never shows a blank page. */
export function parseHash(hash: string): Route {
  const [path, query = ""] = hash.replace(/^#\/?/, "").split("?");
  const parts = path.split("/");
  let project: string | null = null;
  let name = path;
  if (parts[0] === "p" && parts.length === 3) {
    let id = "";
    try { id = decodeURIComponent(parts[1]); } catch { id = ""; }
    project = PROJECT.test(id) ? id : null;
    name = parts[2];
  }
  const screen = canon(name) ?? "orch";
  const params: Record<string, string> = {};
  for (const [k, v] of new URLSearchParams(query)) params[k] = v;
  return { screen, params, project };
}

export function toHash(screen: AnyScreen, params: Record<string, string> = {}, project: string | null = null): string {
  const q = new URLSearchParams(params).toString();
  const view = canon(screen) ?? "orch";
  return `${project ? `#/p/${encodeURIComponent(project)}/` : "#/"}${view}${q ? `?${q}` : ""}`;
}

const subscribe = (cb: () => void) => {
  window.addEventListener("hashchange", cb);
  return () => window.removeEventListener("hashchange", cb);
};

/**
 * The current route and a way to move. The snapshot is the raw hash string, so React sees a stable value. `go` stays in
 * the current project unless it is given one (`null` leaves projects, for a campaign the shell has to place).
 */
export function useRoute(): [Route, (screen: AnyScreen, params?: Record<string, string>, project?: string | null) => void] {
  const hash = useSyncExternalStore(subscribe, () => window.location.hash, () => "");
  const go = useCallback((screen: AnyScreen, params?: Record<string, string>, project?: string | null) => {
    const here = parseHash(window.location.hash).project;
    window.location.hash = toHash(screen, params, project === undefined ? here : project);
  }, []);
  return [parseHash(hash), go];
}
