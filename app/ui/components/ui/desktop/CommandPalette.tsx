import { useEffect, useId, useMemo, useState } from "react";
import { Sheet } from "../base/Sheet";

export type Command = { id: string; label: string; hint?: string; run: () => void };

/** Fuzzy score: every query letter must appear in order; earlier and consecutive matches rank higher. -1 means no match. */
export function score(query: string, label: string): number {
  const q = query.toLowerCase();
  const t = label.toLowerCase();
  let ti = 0;
  let s = 0;
  let streak = 0;
  for (const ch of q) {
    const at = t.indexOf(ch, ti);
    if (at < 0) return -1;
    streak = at === ti ? streak + 1 : 0;
    s += 10 + streak * 5 - Math.min(at, 20);
    ti = at + 1;
  }
  return s;
}

/** Cmd-K palette: a combobox over a listbox, arrows move, Enter runs, Esc closes. Opened and closed by the host. */
export function CommandPalette({ open, onClose, commands, title = "Command palette" }: { open: boolean; onClose: () => void; commands: readonly Command[]; title?: string }) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const base = useId();
  const shown = useMemo(
    () => (q ? commands.map((c) => ({ c, s: score(q, c.label) })).filter((x) => x.s >= 0).sort((a, b) => b.s - a.s).map((x) => x.c) : commands),
    [q, commands],
  );
  useEffect(() => { if (open) { setQ(""); setActive(0); } }, [open]);
  useEffect(() => setActive(0), [q]);
  const run = (c: Command | undefined) => { if (c) { onClose(); c.run(); } };
  return (
    <Sheet open={open} onClose={onClose} title={title}>
      <input
        autoFocus
        role="combobox"
        aria-expanded="true"
        aria-controls={`${base}-list`}
        aria-activedescendant={shown[active] ? `${base}-${shown[active].id}` : undefined}
        aria-label="Search commands"
        className="v-input"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") { e.preventDefault(); setActive((a) => Math.min(a + 1, shown.length - 1)); }
          else if (e.key === "ArrowUp") { e.preventDefault(); setActive((a) => Math.max(a - 1, 0)); }
          else if (e.key === "Enter") { e.preventDefault(); run(shown[active]); }
        }}
      />
      <div role="listbox" id={`${base}-list`} aria-label="Commands">
        {shown.map((c, i) => (
          <div key={c.id} role="option" id={`${base}-${c.id}`} aria-selected={i === active} className="v-row" onMouseEnter={() => setActive(i)} onClick={() => run(c)}>
            <span>{c.label}</span>
            {c.hint && <span className="v-row__meta">{c.hint}</span>}
          </div>
        ))}
        {shown.length === 0 && <p className="v-text v-text--body-sm">No command matches "{q}".</p>}
      </div>
    </Sheet>
  );
}
