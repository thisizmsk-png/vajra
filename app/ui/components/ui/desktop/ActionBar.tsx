import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { Button, Kbd, ariaShortcut } from "../base";

export type Action = {
  id: string;
  label: string;
  /** The chord that runs it, for example ["⌘", "↵"] or ["M"]. Shown, and bound while the bar is mounted. */
  keys?: string[];
  primary?: boolean;
  disabled?: boolean;
  busy?: boolean;
  run: () => void;
};

const MODS: Record<string, "metaKey" | "shiftKey" | "altKey" | "ctrlKey"> = { "⌘": "metaKey", "⇧": "shiftKey", "⌥": "altKey", "⌃": "ctrlKey" };
const KEY: Record<string, string> = { "↵": "Enter", "⌫": "Backspace", "⇥": "Tab", esc: "Escape", "↑": "ArrowUp", "↓": "ArrowDown" };

/** True when the event is exactly this chord. A chord without a modifier never fires while typing in a field. */
export function chordMatches(e: Pick<KeyboardEvent, "key" | "metaKey" | "shiftKey" | "altKey" | "ctrlKey" | "target">, keys: string[]): boolean {
  const mods = keys.filter((k) => k in MODS);
  const rest = keys.filter((k) => !(k in MODS));
  if (rest.length !== 1) return false;
  for (const m of Object.keys(MODS)) if (Boolean(e[MODS[m]]) !== mods.includes(m)) return false;
  if (mods.length === 0) {
    const t = e.target as HTMLElement | null;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return false;
  }
  const want = KEY[rest[0]] ?? rest[0];
  return e.key.toLowerCase() === want.toLowerCase();
}

/**
 * The Graphite action bar (DESIGN.md signature): what is selected, in words, and its actions with their keys,
 * primary first. Only actions the screen can really run belong here.
 */
export function ActionBar({ label, context, actions = [] }: { label: string; context?: ReactNode; actions?: Action[] }) {
  const ref = useRef(actions);
  ref.current = actions;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      const a = ref.current.find((x) => x.keys && chordMatches(e, x.keys));
      if (!a) return;
      e.preventDefault();
      if (!a.disabled && !a.busy) a.run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const sorted = [...actions.filter((a) => a.primary), ...actions.filter((a) => !a.primary)];
  return (
    <footer className="v-actionbar" aria-label={label}>
      <div className="v-actionbar__ctx">{context}</div>
      {sorted.length > 0 && (
        <div className="v-actionbar__acts">
          {sorted.map((a) => (
            <Button key={a.id} variant="ghost" size="sm" className={a.primary ? "v-actionbar__act v-actionbar__act--primary" : "v-actionbar__act"}
              disabled={a.disabled} busy={a.busy} onClick={a.run} aria-keyshortcuts={a.keys ? ariaShortcut(a.keys) : undefined}>
              {a.label}
              {a.keys && <Kbd keys={a.keys} decorative />}
            </Button>
          ))}
        </div>
      )}
    </footer>
  );
}
