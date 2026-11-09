import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api } from "./ipc";
import type { PreferencePatch, PreferenceView, PreferenceEnvelope, PreferenceReceipt } from "./preferenceContract";
import { isPreferenceView } from "./preferenceContract";
import { followSystemTheme } from "./theme";
import { validId } from "./projectContract";
import { S } from "../strings";
const C = S.preferences;
export const SAVED = "vajra-preferences-saved";
/** The preference scope for a project: its id only when the core can store it (a registered T1 id). The shell's
 * folder-derived ids (`p_<hex>`) and the Unsorted bucket read and save global defaults, so a project the core has not
 * registered never fails `preferences_read` or blocks Start (PR 100 review). */
export function preferenceScope(project: { id: string; bucket?: string } | null | undefined): string | null {
  return project && !project.bucket && validId("p", project.id) ? project.id : null;
}
export function usePreferenceScope(projectId: string | null, listenForSaves = true) {
  const scope = projectId ?? ""; const current = useRef(scope); current.current = scope;
  const epoch = useRef(0); const flight = useRef(false);
  type Intent = { envelope: PreferenceEnvelope; patch: PreferencePatch; reset: boolean };
  const pending = useRef<Intent | null>(null);
  const [receipt, setReceipt] = useState<PreferenceReceipt | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const [state, setState] = useState<{ scope: string; view: PreferenceView | null; error: string; busy: boolean }>({ scope, view: null, error: "", busy: false });
  const load = useCallback(() => {
    pending.current = null; setUncertain(false); setReceipt(null);
    const ticket = ++epoch.current;
    setState(s => ({ scope, view: s.scope === scope ? s.view : null, error: "", busy: false }));
    api.preferencesRead(projectId).then(view => { if (ticket === epoch.current && current.current === scope) setState(isPreferenceView(view) && view.project_id === projectId ? { scope, view, error: "", busy: false } : { scope, view: null, error: C.unsupported, busy: false }); }, () => {
      if (ticket === epoch.current && current.current === scope) setState({ scope, view: null, error: C.readFailed, busy: false });
    });
  }, [projectId, scope]);
  useEffect(() => { load(); if (listenForSaves) window.addEventListener(SAVED, load); return () => { ++epoch.current; window.removeEventListener(SAVED, load); }; }, [load, listenForSaves]);
  const view = state.scope === scope ? state.view : null;
  const perform = async (intent: Intent): Promise<boolean> => {
    if (flight.current) return false;
    flight.current = true;
    const ticket = ++epoch.current;
    setState(s => ({ ...s, busy: true, error: "" }));
    setUncertain(false);
    try {
      const out = await api.preferencesAct(intent.envelope, intent.patch, intent.reset);
      if (ticket !== epoch.current || current.current !== scope) return false;
      setReceipt(out.receipt);
      if (!out.receipt || out.receipt.state === "accepted") {
        setUncertain(true);
        throw new Error(C.unconfirmed);
      }
      pending.current = null;
      if (out.receipt.state !== "done") {
        const code = out.receipt.reason ?? out.error?.error;
        throw new Error(code === "revision_conflict"
          ? C.conflict
          : C.rejected(out.receipt.state));
      }
      // Replayed receipts deliberately carry no effect result. Recover it with a read, never a second write.
      const next = out.result ?? await api.preferencesRead(projectId);
      if (ticket !== epoch.current || current.current !== scope) return false;
      if (!isPreferenceView(next) || next.project_id !== projectId) throw new Error(C.unsupportedSave);
      setState({ scope, view: next, error: "", busy: false });
      window.dispatchEvent(new CustomEvent(SAVED));
      return true;
    } catch (e) {
      if (ticket === epoch.current && current.current === scope) {
        if (pending.current) setUncertain(true);
        setState(s => ({ ...s, busy: false, error: e instanceof Error ? e.message : C.saveFailed }));
      }
      return false;
    } finally { flight.current = false; }
  };
  const save = async (patch: PreferencePatch, reset = false) => {
    if (!view || flight.current || pending.current || (receipt && receipt.state !== "done")) return false;
    const intent = { envelope: { action_id: crypto.randomUUID(), project_id: projectId ?? "global", expect_generation: view.rev },
      patch: structuredClone(patch), reset };
    pending.current = intent; setReceipt(null);
    return perform(intent);
  };
  return { view, error: state.scope === scope ? state.error : "", busy: state.scope === scope && state.busy,
    receipt, uncertain, needsReview: !!receipt && receipt.state !== "done" && !uncertain, reload: load, save, retry: () => pending.current ? perform(pending.current) : Promise.resolve(false) };

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
