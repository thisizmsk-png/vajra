import { useEffect, useId, useMemo, useState } from "react";
import { Sheet } from "../base/Sheet";
import { Kbd, ariaShortcut } from "../base/Kbd";
import { Icon } from "../base/Icon";
import { S } from "../../../strings";

/** `keys` is the chord that also runs it elsewhere, shown so people learn it; the palette does not bind it. */
/** `group` "go" files it under "Go to" (a place); anything else is an action. */
export type Command = { id: string; label: string; hint?: string; keys?: string[]; group?: "go"; run: () => void };
type Section = { id: "go" | "act"; label: string; items: Command[] };

/** The result of `score` for a label that does not match. A real match can score below zero (a late letter), so no number such as -1 can mean "no match". */
export const NO_MATCH = Number.NEGATIVE_INFINITY;

/** Fuzzy score: every query letter must appear in order; earlier and consecutive matches rank higher. `NO_MATCH` means no match. */
export function score(query: string, label: string): number {
  const q = query.toLowerCase();
  const t = label.toLowerCase();
  let ti = 0;
  let s = 0;
  let streak = 0;
  for (const ch of q) {
    const at = t.indexOf(ch, ti);
    if (at < 0) return NO_MATCH;
    streak = at === ti ? streak + 1 : 0;
    s += 10 + streak * 5 - Math.min(at, 20);
    ti = at + 1;
  }
  return s;
}

/** Cmd-K palette: a combobox over a listbox, arrows move, Enter runs, Esc closes. Opened and closed by the host. */
export function CommandPalette({ open, onClose, commands, title = S.palette.title }: { open: boolean; onClose: () => void; commands: readonly Command[]; title?: string }) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const base = useId();
  // No query: places under "Go to", then actions, each in its given order. A query ranks one flat list, best first.
  const sections = useMemo<Section[]>(() => {
    if (q) return [{ id: "act", label: "", items: commands.map((c) => ({ c, s: score(q, c.label) })).filter((x) => x.s !== NO_MATCH).sort((a, b) => b.s - a.s).map((x) => x.c) }];
    return [
      { id: "go" as const, label: S.palette.goTo, items: commands.filter((c) => c.group === "go") },
      { id: "act" as const, label: S.palette.actions, items: commands.filter((c) => c.group !== "go") },
    ].filter((s) => s.items.length > 0);
  }, [q, commands]);
  const shown = useMemo(() => sections.flatMap((s) => s.items), [sections]);
  useEffect(() => { if (open) { setQ(""); setActive(0); } }, [open]);
  useEffect(() => setActive(0), [q]);
  useEffect(() => { document.getElementById(`${base}-${shown[active]?.id}`)?.scrollIntoView?.({ block: "nearest" }); }, [active, base, shown]);
  const run = (c: Command | undefined) => { if (c) { onClose(); c.run(); } };
  return (
    <Sheet open={open} onClose={onClose} title={title} className="v-palette">
      <div className="v-palette__field">
        <Icon name="search" />
        <input
          autoFocus
          role="combobox"
          aria-expanded="true"
          aria-autocomplete="list"
          aria-controls={`${base}-list`}
          aria-activedescendant={shown[active] ? `${base}-${shown[active].id}` : undefined}
          aria-label={S.palette.search}
          placeholder={S.palette.search}
          className="v-input"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") { e.preventDefault(); setActive((a) => Math.min(a + 1, shown.length - 1)); }
            else if (e.key === "ArrowUp") { e.preventDefault(); setActive((a) => Math.max(a - 1, 0)); }
            else if (e.key === "Enter" && !e.nativeEvent.isComposing) { e.preventDefault(); run(shown[active]); }
          }}
        />
      </div>
      <div role="listbox" id={`${base}-list`} aria-label={S.palette.list}>
        {sections.map((sec, si) => {
          const start = sections.slice(0, si).reduce((n, s) => n + s.items.length, 0);
          const rows = sec.items.map((c, j) => {
            const i = start + j;
            return (
              <div key={c.id} role="option" id={`${base}-${c.id}`} aria-selected={i === active} aria-keyshortcuts={c.keys ? ariaShortcut(c.keys) : undefined} className="v-row" onMouseMove={() => setActive(i)} onClick={() => run(c)}>
                <span>{c.label}</span>
                {c.hint && <span className="v-row__meta">{c.hint}</span>}
                {c.keys && <Kbd keys={c.keys} decorative />}
              </div>
            );
          });
          if (!sec.label) return rows;
          return (
            <div key={sec.id} role="group" aria-labelledby={`${base}-g-${sec.id}`}>
              <div id={`${base}-g-${sec.id}`} role="presentation" className="v-palette__group">{sec.label}</div>
              {rows}
            </div>
          );
        })}
      </div>
      <p role="status" className="v-text v-text--body-sm">{shown.length === 0 ? S.palette.none(q) : S.palette.count(shown.length)}</p>
    </Sheet>
  );
}
