// The terminal dock: a bottom panel with one tab per terminal, each a user shell in a project folder or a worker
// worktree. Standalone: the app shell mounts `<TerminalDock defaultPlace=... />` once and any screen can call
// `openTerminalHere(place)`. The xterm view is a lazy chunk, so a window that never opens a terminal pays nothing.
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { S } from "../../strings";
import { api, onChanged } from "../ipc";
import type { Lifecycle, TermInfo } from "../ipc";
import type { ViewState } from "./TerminalView";
import "./terminal.css";

const TerminalView = lazy(() => import("./TerminalView"));

export type Kind = "shell" | "agent";
/** `kind` defaults to a plain user shell; an agent terminal is its own tab even in the same folder as a shell. */
export type Place = { cwd: string; label: string; kind?: Kind };

const HERE = "vajra:terminal-here";
/** "Terminal here": open (or focus) a terminal in this project folder or worktree. */
export const openTerminalHere = (place: Place) => window.dispatchEvent(new CustomEvent<Place>(HERE, { detail: place }));

/** `live` is false for a tab restored from the saved layout: it has no view and no shell until it is first shown. */
type Tab = { key: number; cwd: string; label: string; kind: Kind; live: boolean; attachId?: number; gen: number; state: ViewState | { status: "starting" }; /** New output arrived while this tab was not the one in view. */ unread?: boolean };
let seq = 0;

// ---- durable layout --------------------------------------------------------------------------------------------------
// Only the arrangement is kept (folder, label, kind, order, active tab): never output, ids or the shell itself, which
// die with the app (ADR-9). The core still checks every folder against its allowlist when a restored tab starts.
export const LAYOUT_KEY = "vajra:terminal-layout";
const MAX_TABS = 16;
type Saved = { cwd: string; label: string; kind: Kind };
const absolute = (p: unknown): p is string => typeof p === "string" && p.length <= 4096 && /^(\/|[A-Za-z]:[\\/])/.test(p);

export function readLayout(): { tabs: Saved[]; cur: number } {
  try {
    const v = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    if (!v || v.v !== 1 || !Array.isArray(v.tabs)) return { tabs: [], cur: 0 };
    const tabs: Saved[] = [];
    for (const t of v.tabs.slice(0, MAX_TABS)) {
      if (!t || !absolute(t.cwd) || typeof t.label !== "string" || (t.kind !== "shell" && t.kind !== "agent")) return { tabs: [], cur: 0 };
      tabs.push({ cwd: t.cwd, label: t.label.slice(0, 200), kind: t.kind });
    }
    const cur = Number.isInteger(v.cur) ? Math.min(Math.max(v.cur, 0), tabs.length - 1) : 0;
    return { tabs, cur: Math.max(cur, 0) };
  } catch {
    return { tabs: [], cur: 0 };
  }
}
function writeLayout(tabs: Tab[], cur: number | null) {
  try {
    const i = tabs.findIndex((t) => t.key === cur);
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: Math.max(i, 0), tabs: tabs.map((t) => ({ cwd: t.cwd, label: t.label, kind: t.kind })) }));
  } catch {
    // storage full or blocked: the dock works, it just forgets the layout
  }
}
const restore = (): { tabs: Tab[]; cur: number | null } => {
  const l = readLayout();
  const tabs = l.tabs.map((t): Tab => ({ key: ++seq, ...t, live: false, gen: 0, state: { status: "starting" } }));
  return { tabs, cur: tabs[l.cur]?.key ?? null };
};

const base = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
const labelOf = (i: TermInfo) => (i.cwd === i.root ? base(i.root) : `${base(i.root)}/${base(i.cwd)}`);

export function TerminalDock({ defaultPlace }: { defaultPlace?: Place }) {
  const [open, setOpen] = useState(false);
  const [init] = useState(restore);
  const [tabs, setTabs] = useState<Tab[]>(init.tabs);
  const [cur, setCur] = useState<number | null>(init.cur);
  const [focusToken, setFocusToken] = useState(0);
  const [note, setNote] = useState("");
  const [life, setLife] = useState<Lifecycle | null>(null);
  const back = useRef<Element | null>(null);
  const curRef = useRef(cur);
  curRef.current = cur;
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
  /** Open a tab and select it, or at the cap say so and change nothing (no selection of a tab that does not exist). */
  const add = useCallback((p: Place, attachId?: number): number | null => {
    if (tabsRef.current.length >= MAX_TABS) {
      setNote(S.terminal.errors.too_many_terminals);
      return null;
    }
    const key = ++seq;
    const tab: Tab = { key, cwd: p.cwd, label: p.label, kind: p.kind ?? "shell", live: true, attachId, gen: 0, state: { status: "starting" } };
    tabsRef.current = [...tabsRef.current, tab]; // so a second add in the same tick sees the cap too
    setTabs((t) => [...t, tab]);
    setCur(key);
    return key;
  }, []);

  // Terminals outlive the webview (a reload, or a hidden window): pick them up again.
  useEffect(() => {
    let dead = false;
    api.termList().then(
      (r) => {
        if (dead || !r.terminals.length) return;
        // Bind each live shell to the restored tab for its folder (keeping that tab's kind). A live shell is never left
        // without a tab, or nothing could show or close it: it gets one even past the cap, and dormant tabs (no shell,
        // nothing lost) make room for it, never the selected one.
        const next = tabsRef.current.map((t) => ({ ...t }));
        let first: number | null = null;
        for (const i of r.terminals) {
          // The core does not know kinds: prefer the plain shell tab, so a returning shell never lands on an agent tab.
          const t = next.find((n) => !n.live && n.cwd === i.cwd && n.kind === "shell") ?? next.find((n) => !n.live && n.cwd === i.cwd);
          if (t) Object.assign(t, { live: true, attachId: i.id });
          else next.push({ key: ++seq, cwd: i.cwd, label: labelOf(i), kind: "shell", live: true, attachId: i.id, gen: 0, state: { status: "starting" } });
          first ??= (t ?? next[next.length - 1]).key;
        }
        for (let k = next.length - 1; k >= 0 && next.length > MAX_TABS; k--) {
          if (!next[k].live && next[k].key !== curRef.current) next.splice(k, 1);
        }
        setTabs(next);
        if (!tabsRef.current.length && first !== null) setCur(first);
      },
      () => undefined,
    );
    return () => void (dead = true);
  }, [add]);

  // A restored tab starts its shell only when it is first shown, and keeps it after that.
  useEffect(() => {
    if (open && cur !== null) setTabs((all) => (all.some((t) => t.key === cur && !t.live) ? all.map((t) => (t.key === cur ? { ...t, live: true } : t)) : all));
  }, [open, cur]);
  useEffect(() => writeLayout(tabs, cur), [tabs, cur]);

  // Ask the core what a close, a quit or an archive would do: once when the dock is shown and again when a shell starts,
  // ends or goes. Never while hidden, never on a timer.
  const [tick, setTick] = useState(0);
  const refreshLife = useCallback(() => setTick((n) => n + 1), []);
  const shape = tabs.map((t) => `${t.key}:${t.state.status}`).join(",");
  useEffect(() => {
    if (!open) return;
    let stale = false;
    api.lifecycle().then(
      (r) => !stale && setLife(r),
      () => !stale && setLife(null),
    );
    return () => void (stale = true);
  }, [open, shape, tick]);

  // A run starting or ending changes what a close or a quit would do even when no terminal changed: ask again when the app
  // says its runs changed (the same notice the rest of the app listens to), only while the dock is shown, and at most once
  // per burst of notices.
  useEffect(() => {
    if (!open) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = onChanged(() => {
      clearTimeout(timer);
      timer = setTimeout(refreshLife, 250);
    });
    return () => {
      off();
      clearTimeout(timer);
    };
  }, [open, refreshLife]);

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
      const kind = p.kind ?? "shell";
      const have = tabsRef.current.find((t) => t.cwd === p.cwd && t.kind === kind && t.state.status !== "ended");
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
  // Output in a pane nobody is looking at marks its tab until the tab is shown. (Only the mark: the output itself is
  // kept by the core, newest 256 KiB, and arrives when the pane is shown.)
  const onActivity = (key: number) => () => {
    if (openRef.current && curRef.current === key) return;
    patch(key, (t) => (t.unread ? t : { ...t, unread: true }));
  };
  useEffect(() => {
    if (open && cur !== null && tabs.find((t) => t.key === cur)?.unread) setTabs((all) => all.map((t) => (t.key === cur ? { ...t, unread: false } : t)));
  }, [open, cur, tabs]);
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
            const who = t.kind === "agent" ? S.terminal.agentTab(t.label) : t.label;
            const name = t.state.status === "ended" ? S.terminal.endedTab(who) : who;
            return (
              <span key={t.key} className="v-term__tab" data-active={t.key === cur}>
                <button role="tab" id={`v-term-tab-${t.key}`} aria-selected={t.key === cur} aria-label={t.unread ? S.terminal.unreadTab(name) : undefined} tabIndex={t.key === cur ? 0 : -1} title={t.cwd} onClick={() => { setCur(t.key); setFocusToken((n) => n + 1); }}>{name}{t.unread && <i className="v-term__dot" aria-hidden="true" />}</button>
                <button className="v-term__x" aria-label={S.terminal.closeTab(t.label)} onClick={() => close(t.key)}>×</button>
              </span>
            );
          })}
        </div>
        <button className="v-term__add" aria-label={S.terminal.newTab} disabled={!active && !defaultPlace} onClick={() => { add(active ? { cwd: active.cwd, label: base(active.cwd) } : defaultPlace!); setFocusToken((n) => n + 1); }}>+</button>
        <button className="v-term__hide" aria-label={S.terminal.hide} aria-keyshortcuts="Control+Backquote" onClick={hide}>⌄</button>
      </div>
      <LifeLine life={life} refresh={refreshLife} />
      <div className="v-term__body">
        {!tabs.length && (
          <div className="v-term__state">
            <p>{S.terminal.empty}</p>
            <p className="v-term__muted">{S.terminal.emptyHint}</p>
            {defaultPlace && <button className="v-term__btn" onClick={() => add(defaultPlace)}>{S.terminal.openOne}</button>}
          </div>
        )}
        <Suspense fallback={<span className="v-term__muted">{S.states.loading}</span>}>
          {tabs.filter((t) => t.live).map((t) => (
            <TerminalView key={`${t.key}-${t.gen}`} cwd={t.cwd} attachId={t.attachId} active={open && t.key === cur} focusToken={focusToken} onState={onState(t.key)} onActivity={onActivity(t.key)} />
          ))}
        </Suspense>
        {active?.state.status === "starting" && <p className="v-term__badge v-term__muted">{S.terminal.starting}</p>}
        {active?.state.status === "error" && (
          <div className="v-term__state v-term__state--over" role="alert">
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

const nameOf = (p: string) => base(p);

/** One honest line from the core's own report, what the last crash left behind, and the shells behind it. Nothing renders when nothing is affected. */
function LifeLine({ life, refresh }: { life: Lifecycle | null; refresh: () => void }) {
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [recovery, setRecovery] = useState("");
  if (!life) return null;
  const { terminals: t, runs: r } = life.close_keeps;
  const keeps = [t && S.terminal.lifecycle.terminals(t), r && S.terminal.lifecycle.runs(r)].filter(Boolean).join(" and ");
  const leaves = life.terminals.filter((x) => life.archive_leaves.includes(x.id)).map((x) => nameOf(x.cwd)).join(", ");
  const owned = life.found.filter((f) => f.verdict === "owned");
  const reused = life.found.filter((f) => f.verdict === "reused");
  if (!keeps && !leaves && !owned.length && !reused.length && !recovery) return null;
  const L = S.terminal.lifecycle;
  // Say what happened: only a result that is exactly "ended" counts as ended.
  const end = () => {
    setRecovery("");
    api.recoverOrphans(owned.map((f) => f.pid)).then(
      (r) => setRecovery(r.results.every((x) => x.result === "ended") ? "" : typeof r.results.find((x) => typeof x.result === "object") !== "undefined" ? L.stillRunning : L.endFailed),
      () => setRecovery(L.endFailed),
    ).finally(refresh);
  };
  const ask = () => {
    setFailed(false);
    api.diagnosticsPreview().then((d) => setText(d.text), () => (setText(null), setFailed(true)));
  };
  return (
    <div className="v-term__life" data-testid="v-term-lifecycle">
      <p>
        {keeps && L.keeps(keeps, t + r > 1 || (t > 0 && r > 0))} {leaves && L.archive(leaves)}
      </p>
      {recovery && <p role="alert">{recovery}</p>}
      {(owned.length > 0 || reused.length > 0) && (
        <div data-testid="v-term-found">
          <p>{L.foundTitle}</p>
          <ul>
            {owned.map((f) => <li key={`${f.pid}-${f.stamp}`}>{L.foundOwned(f.pid, f.cwd, f.members ?? 1)}</li>)}
            {reused.map((f) => <li key={`${f.pid}-${f.stamp}`}>{L.foundReused(f.pid)}</li>)}
          </ul>
          {owned.length > 0 && (
            <button className="v-term__btn" onClick={end}>{L.endLeft(owned.length)}</button>
          )}
        </div>
      )}
      <details>
        <summary>{L.details}</summary>
        <ul>
          {life.terminals.map((x) => (
            <li key={x.id}>
              {x.cwd} · {L.pid(x.pid)} · {x.alive ? L.running : L.endedState}
            </li>
          ))}
          {life.runs.map((x) => <li key={x.stream}>{L.runGroup(x.pid)}</li>)}
        </ul>
        <button className="v-term__btn" onClick={ask}>{L.preview}</button>
        {failed && <p role="alert">{L.previewFailed}</p>}
        {text !== null && (
          <>
            <pre data-testid="v-term-diag" tabIndex={0}>{text}</pre>
            <p className="v-term__muted">{L.previewNote}</p>
            <button className="v-term__btn" onClick={() => void navigator.clipboard?.writeText(text)}>{L.copy}</button>
          </>
        )}
      </details>
    </div>
  );
}
