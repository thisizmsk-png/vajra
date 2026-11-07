import type { ReactNode } from "react";
import { Kbd, Mark } from "../shared";
import { repo, run } from "../data";

const NAV = [
  { id: "home", label: "Home", key: "1" },
  { id: "crew", label: "Crew", key: "2" },
  { id: "inbox", label: "Inbox", key: "3", count: 1 },
  { id: "chat", label: "Chat with Lead", key: "4" },
  { id: "history", label: "History", key: "5" },
];

/** Sidebar + main + the action bar. `bar` is what the action bar shows for the current selection. */
export function ShellA({ current, children, bar }: { current: string; children: ReactNode; bar: ReactNode }) {
  return (
    <div className="a-app">
      <nav className="a-side" aria-label="Main">
        <div className="a-ws">
          <Mark /> Vajra <span className="sub">{repo.name}</span>
        </div>
        {NAV.map((n) => (
          <a key={n.id} className="a-nav" href={`#${n.id}`} aria-current={n.id === current ? "page" : undefined}>
            {n.label}
            {n.count ? <span className="count" aria-label={`${n.count} waiting`}>{n.count}</span> : null}
            <Kbd>⌘{n.key}</Kbd>
          </a>
        ))}
        <div className="a-group">Running</div>
        <a className="a-liverow" href="#crew">
          <span className="a-livedot" aria-hidden="true" />
          <span>
            <div className="t">{run.title}</div>
            <div className="s">{run.phase}, round {run.round}. 1 waits for you</div>
          </span>
        </a>
        <div className="spacer" />
        <a className="a-nav" href="#settings">Settings <Kbd>⌘,</Kbd></a>
      </nav>
      <main className="a-main">{children}</main>
      <footer className="a-bar" aria-label="Actions">{bar}</footer>
    </div>
  );
}
