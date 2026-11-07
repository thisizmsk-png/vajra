import type { ReactNode } from "react";
import { Kbd, Mark, StatusDot } from "../shared";
import { ago, history, repo, run } from "../data";
import { IcChevron, IcHistory, IcHome, IcInbox, IcPlus, IcSearch, IcSettings, IcShield } from "./icons";

const outcome = { Delivered: "done", Stopped: "blocked", Failed: "failed" } as const;

/**
 * Graphite v2 shell. From the competitor pass: runs are first-class in the sidebar with a live glyph and a
 * relative time (AO, Orca), search and commands share one field (Raycast, AO), and the action bar stays the
 * signature. `current` is "home", "inbox", "history" or "run" (the live run's Crew).
 */
export function ShellG({ current, bar, children, inbox = 2, empty = false }: { current: string; bar: ReactNode; children: ReactNode; inbox?: number; empty?: boolean }) {
  return (
    <div className="g-app g">
      <nav className="g-side" aria-label="Main">
        <div className="g-ws">
          <span className="mark"><Mark /></span> Vajra
          <button className="repo" aria-label={`Repository ${repo.name}, change`}>{repo.name}<IcChevron /></button>
        </div>
        <button className="g-search" aria-label="Search or run a command"><IcSearch /><span className="t">Search or run</span><span className="g-keys"><Kbd>⌘</Kbd><Kbd>K</Kbd></span></button>
        <a className="g-nav" href="#home" aria-current={current === "home" ? "page" : undefined}><IcHome />Home<Kbd>⌘1</Kbd></a>
        <a className="g-nav" href="#inbox" aria-current={current === "inbox" ? "page" : undefined}>
          <IcInbox />Inbox{inbox ? <span className="count" aria-label={`${inbox} waiting for you`}>{inbox}</span> : <Kbd>⌘2</Kbd>}
        </a>
        <a className="g-nav" href="#history" aria-current={current === "history" ? "page" : undefined}><IcHistory />History<Kbd>⌘3</Kbd></a>

        <div className="g-grouph"><span>Runs in {repo.name}</span><button aria-label="New run, Command N"><IcPlus /></button></div>
        <div className="g-runs">
          {empty ? (
            <div className="g-empty" style={{ padding: "4px 8px", fontSize: 12 }}>No runs yet</div>
          ) : (
            <>
              <a className="g-run" href="#crew" aria-current={current === "run" ? "page" : undefined}>
                <span className="g-spin" role="img" aria-label="Running" />
                <span className="t">{run.title}</span>
                <span className="r needs" aria-label="2 waiting for you">2</span>
              </a>
              {history.map((h) => (
                <a key={h.id} className="g-run g-dot" href="#history">
                  <StatusDot status={outcome[h.outcome]} label={h.outcome} />
                  <span className="t">{h.title}</span>
                  <span className="r">{ago[h.id]}</span>
                </a>
              ))}
            </>
          )}
        </div>
        <div className="spacer" />
        <a className="g-nav" href="#settings"><IcSettings />Settings<Kbd>⌘,</Kbd></a>
        <div className="g-health" aria-label="Health">
          <span className="ok"><IcShield />Guard on</span>
          <span>Engine ok</span>
          <span className="num">App {run.memory}</span>
        </div>
      </nav>
      <main className="g-main">{children}</main>
      <footer className="g-bar" aria-label="Actions for the selection">{bar}</footer>
    </div>
  );
}

/** One action in the bar: label plus its keys. */
export function Act({ label, keys, primary }: { label: string; keys: string[]; primary?: boolean }) {
  return (
    <button className={`act${primary ? " act--primary" : ""}`}>
      <span className="lbl">{label}</span>
      <span className="g-keys">{keys.map((k) => <Kbd key={k}>{k}</Kbd>)}</span>
    </button>
  );
}
