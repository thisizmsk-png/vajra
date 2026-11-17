import { cn } from "../cn";

/** Spoken names for the symbol keys, so a screen reader says "Command Return", not two unknown symbols. */
const NAMES: Record<string, string> = {
  "⌘": "Command", "⇧": "Shift", "⌥": "Option", "⌃": "Control", "↵": "Return", "⌫": "Delete", "⇥": "Tab",
  "↑": "Up", "↓": "Down", "←": "Left", "→": "Right", esc: "Escape",
};
export const keyName = (k: string) => NAMES[k] ?? k;

/** The same chord in aria-keyshortcuts syntax ("Meta+Enter"), for the control the chord runs. */
const ARIA: Record<string, string> = { "⌘": "Meta", "⇧": "Shift", "⌥": "Alt", "⌃": "Control", "↵": "Enter", "⌫": "Backspace", "⇥": "Tab", esc: "Escape", "↑": "ArrowUp", "↓": "ArrowDown", "←": "ArrowLeft", "→": "ArrowRight" };
export const ariaShortcut = (keys: string[]) => keys.map((k) => ARIA[k] ?? k).join("+");

/**
 * A key chord shown next to an action (DESIGN.md: keys are visible, so people learn them by using the app).
 * Inside a control pass `decorative`: the chord is hidden from the accessible name, and the control carries
 * `aria-keyshortcuts={ariaShortcut(keys)}` instead.
 * A chord is one keycap ("⌘1"), the way the menu bar writes it; a word key ("esc") keeps a space from its neighbour.
 */
export const chordText = (keys: string[]) => keys.reduce((s, k, i) => (i > 0 && (k.length > 1 || keys[i - 1].length > 1) ? `${s} ${k}` : s + k), "");

export function Kbd({ keys, decorative, className }: { keys: string[]; decorative?: boolean; className?: string }) {
  return (
    <kbd className={cn("v-kbd-group", className)} aria-label={decorative ? undefined : keys.map(keyName).join(" ")} aria-hidden={decorative || undefined}>
      <kbd className="v-kbd" aria-hidden="true">{chordText(keys)}</kbd>
    </kbd>
  );
}
