import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";

/**
 * A listbox of `ListRow` options. One tab stop: the listbox itself, which keeps focus and points at the active option
 * with `aria-activedescendant`. Up/Down/Home/End move the active option, Enter or Space selects it. When there is no
 * valid active option (first focus, or the active row was removed) the selected option, else the first, becomes active.
 */
export function ListBox({ label, children }: { label: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<string | undefined>();
  const options = () => Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? []);
  const initial = () => options().find((o) => o.getAttribute("aria-selected") === "true") ?? options()[0];
  const show = (o: HTMLElement | undefined) => {
    setActive(o?.id);
    o?.scrollIntoView?.({ block: "nearest" });
  };
  useEffect(() => {
    const all = options();
    if (active && !all.some((o) => o.id === active)) show(initial()); // the active row went away
    for (const o of all) o.toggleAttribute("data-active", o.id === active);
  });
  const go = (to: (i: number, n: number) => number) => {
    const all = options();
    const now = all.findIndex((o) => o.id === active);
    show(now < 0 ? initial() : all[Math.min(all.length - 1, Math.max(0, to(now, all.length)))]);
  };
  const key = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") go((i) => i + 1);
    else if (e.key === "ArrowUp") go((i) => i - 1);
    else if (e.key === "Home") go(() => 0);
    else if (e.key === "End") go((_, n) => n - 1);
    else if (e.key === "Enter" || e.key === " ") options().find((o) => o.id === active)?.click();
    else return;
    e.preventDefault();
  };
  return (
    <div
      ref={ref}
      role="listbox"
      aria-label={label}
      aria-activedescendant={active}
      tabIndex={0}
      className="v-listbox"
      onKeyDown={key}
      // A pointer selection moves the active option too, so Enter and Space act on what was clicked.
      onClick={(e) => {
        const o = (e.target as HTMLElement).closest<HTMLElement>('[role="option"]');
        if (o) {
          setActive(o.id);
          ref.current?.focus();
        }
      }}
      onFocus={() => {
        if (!options().some((o) => o.id === active)) show(initial());
      }}
    >
      {children}
    </div>
  );
}
