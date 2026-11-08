// SPIKE: bottom dock with one tab per terminal. Toggle with Ctrl+` . The xterm view is a lazy chunk.
import { lazy, Suspense, useEffect, useState } from "react";
import "./terminal.css";

const TerminalView = lazy(() => import("./TerminalView"));

type Tab = { key: number; cwd: string; label: string; exited?: boolean };
let seq = 0;

export function TerminalDock({ defaultCwd }: { defaultCwd: string }) {
  const [open, setOpen] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [cur, setCur] = useState<number | null>(null);
  const add = () => {
    const key = ++seq;
    setTabs((t) => [...t, { key, cwd: defaultCwd, label: defaultCwd.split("/").filter(Boolean).pop() ?? "terminal" }]);
    setCur(key);
    setOpen(true);
  };
  const close = (key: number) => {
    setTabs((t) => {
      const next = t.filter((x) => x.key !== key);
      setCur((c) => (c === key ? (next[next.length - 1]?.key ?? null) : c));
      return next;
    });
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "`") {
        e.preventDefault();
        if (!tabs.length) add();
        else setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  if (!tabs.length && !open) return null;
  return (
    <section className="v-term" aria-label="Terminal" hidden={!open}>
      <div className="v-term__bar" role="tablist" aria-label="Terminals">
        {tabs.map((t) => (
          <span key={t.key} className="v-term__tab" data-active={t.key === cur}>
            <button role="tab" aria-selected={t.key === cur} onClick={() => setCur(t.key)}>{t.label}{t.exited ? " (ended)" : ""}</button>
            <button aria-label={`Close ${t.label}`} onClick={() => close(t.key)}>×</button>
          </span>
        ))}
        <button className="v-term__add" aria-label="New terminal" onClick={add}>+</button>
        <button className="v-term__hide" aria-label="Hide terminal" onClick={() => setOpen(false)}>⌄</button>
      </div>
      <div className="v-term__body">
        <Suspense fallback={<span className="v-term__loading">Loading terminal</span>}>
          {tabs.map((t) => (
            <TerminalView key={t.key} cwd={t.cwd} active={t.key === cur} onExit={() => setTabs((x) => x.map((y) => (y.key === t.key ? { ...y, exited: true } : y)))} />
          ))}
        </Suspense>
      </div>
    </section>
  );
}
