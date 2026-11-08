import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api } from "./ipc";
import type { PreferencePatch, PreferenceView } from "./preferenceContract";
import { isPreferenceView } from "./preferenceContract";
import { followSystemTheme } from "./theme";
export const SAVED = "vajra-preferences-saved";
export function usePreferenceScope(projectId: string | null, listenForSaves = true) {
  const scope = projectId ?? ""; const current = useRef(scope); current.current = scope;
  const epoch = useRef(0); const flight = useRef(false);
  const [state, setState] = useState<{ scope: string; view: PreferenceView | null; error: string; busy: boolean }>({ scope, view: null, error: "", busy: false });
  const load = useCallback(() => {
    const ticket = ++epoch.current;
    setState(s => ({ scope, view: s.scope === scope ? s.view : null, error: "", busy: false }));
    api.preferencesRead(projectId).then(view => { if (ticket === epoch.current && current.current === scope) setState(isPreferenceView(view) && view.project_id === projectId ? { scope, view, error: "", busy: false } : { scope, view: null, error: "The core returned unsupported preferences. Update the desktop and reload.", busy: false }); }, () => {
      if (ticket === epoch.current && current.current === scope) setState({ scope, view: null, error: "Preferences could not be read. The saved file was preserved.", busy: false });
    });
  }, [projectId, scope]);
  useEffect(() => { load(); if (listenForSaves) window.addEventListener(SAVED, load); return () => { ++epoch.current; window.removeEventListener(SAVED, load); }; }, [load, listenForSaves]);
  const view = state.scope === scope ? state.view : null;
  const save = async (patch: PreferencePatch, reset = false): Promise<boolean> => {
    if (!view || flight.current) return false;
    flight.current = true; const ticket = ++epoch.current; setState(s => ({ ...s, busy: true, error: "" }));
    try {
      const next = await api.preferencesUpdate(projectId, view.rev, patch, reset);
      if (ticket !== epoch.current || current.current !== scope) return false;
      if (!isPreferenceView(next) || next.project_id !== projectId) throw new Error("Unsupported preferences response");
      setState({ scope, view: next, error: "", busy: false }); window.dispatchEvent(new CustomEvent(SAVED)); return true;
    } catch (e) {
      if (ticket === epoch.current && current.current === scope) setState(s => ({ ...s, busy: false, error: (e as { error?: string })?.error === "revision_conflict"
        ? "Preferences changed in another window. Reload and review before saving again." : "Preferences were not saved. Reload to check the current values." }));
      return false;
    } finally { flight.current = false; }
  };
  return { view, error: state.scope === scope ? state.error : "", busy: state.scope === scope && state.busy, reload: load, save };
}
const Context = createContext<{ connected: boolean; view: PreferenceView | null; error: string }>({ connected: false, view: null, error: "" });
export const usePreferences = () => useContext(Context);
export function PreferencesProvider({ projectId, children }: { projectId: string | null; children: ReactNode }) {
  const prefs = usePreferenceScope(projectId);
  const theme = prefs.view?.effective.theme;
  useEffect(() => {
    if (!theme) return followSystemTheme();
    if (theme === "system") return followSystemTheme();
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  return <Context.Provider value={{ connected: true, view: prefs.view, error: prefs.error }}>{children}</Context.Provider>;
}
/** A new launch owns its own copy, not a mutable pointer into settings. Live sessions never subscribe to it. */
export function capturePreferences(view: PreferenceView) {
  return Object.freeze({ revision: view.rev, settings: Object.freeze({ ...view.effective, shortcuts: Object.freeze({ ...view.effective.shortcuts }) }) });
}
