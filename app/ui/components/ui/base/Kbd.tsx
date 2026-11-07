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
 */
export function Kbd({ keys, decorative, className }: { keys: string[]; decorative?: boolean; className?: string }) {
  return (
    <kbd className={cn("v-kbd-group", className)} aria-label={decorative ? undefined : keys.map(keyName).join(" ")} aria-hidden={decorative || undefined}>
      {keys.map((k) => (
        <kbd key={k} className="v-kbd" aria-hidden="true">{k}</kbd>
      ))}
    </kbd>
  );
}
