// The terminal dock: a bottom panel with one tab per terminal, each a user shell in a project folder or a worker
// worktree. Standalone: the app shell mounts `<TerminalDock defaultPlace=... />` once and any screen can call
// `openTerminalHere(place)`. The xterm view is a lazy chunk, so a window that never opens a terminal pays nothing.
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { S } from "../../strings";
import { api } from "../ipc";
import type { TermInfo } from "../ipc";
import type { ViewState } from "./TerminalView";
import "./terminal.css";

const TerminalView = lazy(() => import("./TerminalView"));

export type Place = { cwd: string; label: string };

const HERE = "vajra:terminal-here";
/** "Terminal here": open (or focus) a terminal in this project folder or worktree. */
export const openTerminalHere = (place: Place) => window.dispatchEvent(new CustomEvent<Place>(HERE, { detail: place }));

type Tab = { key: number; cwd: string; label: string; attachId?: number; gen: number; state: ViewState | { status: "starting" } };
let seq = 0;

const base = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
const labelOf = (i: TermInfo) => (i.cwd === i.root ? base(i.root) : `${base(i.root)}/${base(i.cwd)}`);

export function TerminalDock({ defaultPlace }: { defaultPlace?: Place }) {
  const [open, setOpen] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [cur, setCur] = useState<number | null>(null);
  const [focusToken, setFocusToken] = useState(0);
  const [note, setNote] = useState("");
  const back = useRef<Element | null>(null);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const openRef = useRef(open);
  openRef.current = open;
  const placeRef = useRef(defaultPlace);
  placeRef.current = defaultPlace;

  const show = useCallback(() => {
    if (!openRef.current) back.current = document.activeElement;
    setOpen(true);
    setFocusToken((t) => t + 1);
  }, []);
  const hide = useCallback(() => {
    setOpen(false);
    const el = back.current;
    back.current = null;
    if (el instanceof HTMLElement && el.isConnected) el.focus();
  }, []);
  const add = useCallback((p: Place, attachId?: number) => {
    const key = ++seq;
    setTabs((t) => [...t, { key, cwd: p.cwd, label: p.label, attachId, gen: 0, state: { status: "starting" } }]);
    setCur(key);
    return key;
  }, []);

  // Terminals outlive the webview (a reload, or a hidden window): pick them up again.
  useEffect(() => {
    let dead = false;
    api.termList().then(
      (r) => {
        if (dead || !r.terminals.length) return;
        const keys = r.terminals.map((i) => add({ cwd: i.cwd, label: labelOf(i) }, i.id));
        setCur(keys[0]);
      },
      () => undefined,
    );
    return () => void (dead = true);
  }, [add]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && !e.metaKey && !e.altKey && (e.key === "`" || e.code === "Backquote")) {
        e.preventDefault();
        e.stopPropagation();
        if (openRef.current) hide();
        else {
          if (!tabsRef.current.length && placeRef.current) add(placeRef.current);
          show();
        }
      }
    };
    const onHere = (e: Event) => {
      const p = (e as CustomEvent<Place>).detail;
      const have = tabsRef.current.find((t) => t.cwd === p.cwd && t.state.status !== "ended");
      if (have) setCur(have.key);
      else add(p);
      show();
    };
    window.addEventListener("keydown", onKey, true); // capture: xterm stops the key before the window sees it otherwise
    window.addEventListener(HERE, onHere);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener(HERE, onHere);
    };
  }, [add, hide, show]);

  const close = (key: number) => {
    const i = tabs.findIndex((t) => t.key === key);
    const next = tabs.filter((t) => t.key !== key);
    setTabs(next);
    if (cur === key) setCur(next[Math.min(i, next.length - 1)]?.key ?? null);
    if (!next.length) hide();
  };
  const patch = (key: number, f: (t: Tab) => Tab) => setTabs((all) => all.map((t) => (t.key === key ? f(t) : t)));
  const onState = (key: number) => (s: ViewState) => {
    if (s.status === "busy") return setNote(S.terminal.busy);
    patch(key, (t) => ({ ...t, state: s }));
    if (s.status === "ended") setNote(S.terminal.announceEnded(tabsRef.current.find((t) => t.key === key)?.label ?? S.terminal.region));
  };
  const restart = (key: number) => patch(key, (t) => ({ ...t, attachId: undefined, gen: t.gen + 1, state: { status: "starting" } }));

  const onTabKey = (e: React.KeyboardEvent) => {
    const i = tabs.findIndex((t) => t.key === cur);
    const go = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : null;
    if (go === null || !tabs.length) return;
    e.preventDefault();
    const t = tabs[(go + tabs.length) % tabs.length];
    setCur(t.key);
    requestAnimationFrame(() => document.getElementById(`v-term-tab-${t.key}`)?.focus());
  };

  if (!open && !tabs.length) return null;
  const active = tabs.find((t) => t.key === cur);
  return (
    <section className="v-term" aria-label={S.terminal.region} aria-describedby="v-term-hint" hidden={!open}>
      <p id="v-term-hint" className="v-term__sr">{S.terminal.leaveHint}</p>
      <div className="v-term__bar">
        <div role="tablist" aria-label={S.terminal.tabs} className="v-term__tabs" onKeyDown={onTabKey}>
          {tabs.map((t) => {
            const name = t.state.status === "ended" ? S.terminal.endedTab(t.label) : t.label;
            return (
              <span key={t.key} className="v-term__tab" data-active={t.key === cur}>
                <button role="tab" id={`v-term-tab-${t.key}`} aria-selected={t.key === cur} tabIndex={t.key === cur ? 0 : -1} title={t.cwd} onClick={() => { setCur(t.key); setFocusToken((n) => n + 1); }}>{name}</button>
                <button className="v-term__x" aria-label={S.terminal.closeTab(t.label)} onClick={() => close(t.key)}>×</button>
              </span>
            );
          })}
        </div>
        <button className="v-term__add" aria-label={S.terminal.newTab} disabled={!active && !defaultPlace} onClick={() => { add(active ? { cwd: active.cwd, label: active.label } : defaultPlace!); setFocusToken((n) => n + 1); }}>+</button>
        <button className="v-term__hide" aria-label={S.terminal.hide} aria-keyshortcuts="Control+Backquote" onClick={hide}>⌄</button>
      </div>
      <div className="v-term__body">
        {!tabs.length && (
          <div className="v-term__state">
            <p>{S.terminal.empty}</p>
            <p className="v-term__muted">{S.terminal.emptyHint}</p>
            {defaultPlace && <button className="v-term__btn" onClick={() => add(defaultPlace)}>{S.terminal.openOne}</button>}
          </div>
        )}
        <Suspense fallback={<span className="v-term__muted">{S.states.loading}</span>}>
          {tabs.map((t) => (
            <TerminalView key={`${t.key}-${t.gen}`} cwd={t.cwd} attachId={t.attachId} active={open && t.key === cur} focusToken={focusToken} onState={onState(t.key)} />
          ))}
        </Suspense>
        {active?.state.status === "starting" && <p className="v-term__badge v-term__muted">{S.terminal.starting}</p>}
        {active?.state.status === "error" && (
          <div className="v-term__state" role="alert">
            <p>{S.terminal.errors[active.state.code] ?? active.state.detail}</p>
            <button className="v-term__btn" onClick={() => restart(active.key)}>{S.terminal.retry}</button>
          </div>
        )}
        {active?.state.status === "ended" && (
          <div className="v-term__ended">
            <span>{S.terminal.ended(active.state.code)}</span>
            <button className="v-term__btn" onClick={() => restart(active.key)}>{S.terminal.restart}</button>
          </div>
        )}
      </div>
      <p role="status" className="v-term__sr">{note}</p>
    </section>
  );
}
