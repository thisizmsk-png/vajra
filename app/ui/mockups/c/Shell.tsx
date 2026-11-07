import type { ReactNode } from "react";
import { Kbd, Mark } from "../shared";
import { repo, run } from "../data";

export function ShellC({ current, children }: { current: string; children: ReactNode }) {
  const nav = [
    { id: "home", label: "Home" },
    { id: "crew", label: "Crew" },
    { id: "inbox", label: "Inbox", count: 1 },
    { id: "chat", label: "Lead" },
    { id: "history", label: "History" },
  ];
  return (
    <div className="c-app">
      <header className="c-top">
        <span className="c-word"><Mark size={18} /> Vajra</span>
        <nav className="c-nav" aria-label="Main">
          {nav.map((n) => (
            <a key={n.id} href={`#${n.id}`} aria-current={n.id === current ? "page" : undefined}>
              {n.label}{n.count ? <span className="count" aria-label={`${n.count} waiting`}>{n.count}</span> : null}
            </a>
          ))}
        </nav>
        <div className="r">
          <span>{repo.name}</span>
          <span className="num">App {run.memory}</span>
          <span>Guard on</span>
          <span><Kbd>⌘K</Kbd></span>
        </div>
      </header>
      {children}
    </div>
  );
}
