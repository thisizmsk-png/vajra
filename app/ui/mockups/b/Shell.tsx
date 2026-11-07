import type { ReactNode } from "react";
import { Kbd, Mark } from "../shared";
import { history, repo, run } from "../data";

const glyph = { Delivered: { g: "✓", c: "pass" }, Stopped: { g: "■", c: "sec" }, Failed: { g: "✕", c: "fail" } } as const;

/** Tabs across the top (like terminal tabs), sessions down the left, a status line at the bottom. */
export function ShellB({ current, children }: { current: "home" | "crew"; children: ReactNode }) {
  return (
    <div className="b-app">
      <header className="b-top">
        <div className="b-brand"><Mark /> Vajra</div>
        <div className="b-tabs" role="tablist" aria-label="Open views">
          <button role="tab" aria-selected={current === "home"} className="b-tab">new run</button>
          <button role="tab" aria-selected={current === "crew"} className="b-tab"><span className="dot" aria-hidden="true" />{repo.name}: rate limiting <span className="wait" aria-label="1 waiting for you">▲ 1</span></button>
        </div>
        <div className="r">
          <button className="b-icon">Inbox <Kbd>⌘3</Kbd></button>
          <button className="b-icon">Lead <Kbd>⌘L</Kbd></button>
          <button className="b-icon">Palette <Kbd>⌘K</Kbd></button>
        </div>
      </header>
      <div className="b-mid">
        <nav className="b-rail" aria-label="Runs">
          <h2>Runs <span><Kbd>⌘N</Kbd></span></h2>
          <a className="b-ses" href="#crew" aria-current={current === "crew"}>
            <span className="g live">●</span>
            <span className="t">{run.title}</span>
            <span className="s">{run.phase.toLowerCase()} r{run.round}, {run.elapsed}, ▲ 1</span>
          </a>
          {history.map((h) => (
            <a key={h.id} className="b-ses" href="#history">
              <span className={`g ${glyph[h.outcome].c}`}>{glyph[h.outcome].g}</span>
              <span className="t">{h.title}</span>
              <span className="s">{h.outcome.toLowerCase()}, {h.passing} pass</span>
            </a>
          ))}
          <div className="spacer" />
          <div className="b-railnav">
            <a href="#home" aria-current={current === "home" ? "page" : undefined}>Home <Kbd>⌘1</Kbd></a>
            <a href="#inbox">Inbox <span className="count">1</span></a>
            <a href="#history">History <Kbd>⌘5</Kbd></a>
            <a href="#settings">Settings <Kbd>⌘,</Kbd></a>
          </div>
        </nav>
        {children}
      </div>
      <footer className="b-status">
        <span>{current === "crew" ? `${run.id}  ${run.phase.toLowerCase()} round ${run.round}/${run.maxRounds}` : `${repo.path}  ${repo.base}@${repo.head}`}</span>
        <span>{run.workers} workers live</span>
        <div className="r">
          <span>app {run.memory}</span>
          <span className="on">guard on</span>
          <span>sandbox off</span>
          <span>engine ok</span>
        </div>
      </footer>
    </div>
  );
}
