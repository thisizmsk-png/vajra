// Dev only (`#terminal`): the dock on its own, with buttons standing in for the "Terminal here" actions of a project
// and a worktree. Not an app screen; the production build drops it.
import { TerminalDock, openTerminalHere } from "./TerminalDock";

const PLACES = [
  { cwd: "/work/app", label: "app" },
  { cwd: "/work/app/.claude/worktrees/w1", label: "app/w1" },
  { cwd: "/work/denied", label: "denied" },
];

export function TerminalHarness() {
  return (
    <main style={{ padding: 24 }}>
      <h1>Terminal dock</h1>
      <p>Press Control and the backtick key, or open a terminal in a place:</p>
      <div style={{ display: "flex", gap: 8 }}>
        {PLACES.map((p) => (
          <button key={p.cwd} onClick={() => openTerminalHere(p)}>{`Terminal here: ${p.label}`}</button>
        ))}
      </div>
      <TerminalDock defaultPlace={PLACES[0]} />
    </main>
  );
}
