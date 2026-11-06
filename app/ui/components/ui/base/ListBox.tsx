import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";

/**
 * A listbox of `ListRow` options. One tab stop: the listbox itself, which keeps focus and points at the active option
 * with `aria-activedescendant`. Up/Down/Home/End move the active option, Enter or Space selects it.
 */
export function ListBox({ label, children }: { label: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<string | undefined>();
  const options = () => Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? []);
  useEffect(() => {
    for (const o of options()) o.toggleAttribute("data-active", o.id === active);
  });
  const go = (to: (i: number, n: number) => number) => {
    const all = options();
    if (!all.length) return;
    const now = Math.max(0, all.findIndex((o) => o.id === active));
    const next = all[Math.min(all.length - 1, Math.max(0, to(now, all.length)))];
    setActive(next.id);
    next.scrollIntoView?.({ block: "nearest" });
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
      onFocus={() => {
        if (!active) setActive((options().find((o) => o.getAttribute("aria-selected") === "true") ?? options()[0])?.id);
      }}
    >
      {children}
    </div>
  );
}
