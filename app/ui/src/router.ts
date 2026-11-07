// A hash router with no dependency: `#/run?slug=a&id=b`. A hash needs no server and survives a reload in the webview.
import { useCallback, useSyncExternalStore } from "react";

export const SCREENS = ["home", "run", "crew", "chat", "inbox", "history", "settings"] as const;
export type Screen = (typeof SCREENS)[number];
export type Route = { screen: Screen; params: Record<string, string> };

const isScreen = (s: string): s is Screen => (SCREENS as readonly string[]).includes(s);

/** Anything that is not a known screen is Home: a stale or hand-typed hash never shows a blank page. */
export function parseHash(hash: string): Route {
  const [path, query = ""] = hash.replace(/^#\/?/, "").split("?");
  const screen = isScreen(path) ? path : "home";
  const params: Record<string, string> = {};
  for (const [k, v] of new URLSearchParams(query)) params[k] = v;
  return { screen, params };
}

export function toHash(screen: Screen, params: Record<string, string> = {}): string {
  const q = new URLSearchParams(params).toString();
  return `#/${screen}${q ? `?${q}` : ""}`;
}

const subscribe = (cb: () => void) => {
  window.addEventListener("hashchange", cb);
  return () => window.removeEventListener("hashchange", cb);
};

/** The current route and a way to move. The snapshot is the raw hash string, so React sees a stable value. */
export function useRoute(): [Route, (screen: Screen, params?: Record<string, string>) => void] {
  const hash = useSyncExternalStore(subscribe, () => window.location.hash, () => "");
  const go = useCallback((screen: Screen, params?: Record<string, string>) => {
    window.location.hash = toHash(screen, params);
  }, []);
  return [parseHash(hash), go];
}
