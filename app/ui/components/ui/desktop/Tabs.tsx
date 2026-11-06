import { useId, useRef } from "react";
import type { KeyboardEvent, ReactNode } from "react";

/** Tablist with roving tabindex: Left/Right (and Home/End) move and activate; the panel is labelled by its tab. */
export function Tabs<V extends string>({ label, value, tabs, onChange }: { label: string; value: V; tabs: ReadonlyArray<{ id: V; label: string; panel: ReactNode }>; onChange: (v: V) => void }) {
  const base = useId();
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const move = (e: KeyboardEvent, i: number) => {
    const n = tabs.length;
    const to = e.key === "ArrowRight" ? (i + 1) % n : e.key === "ArrowLeft" ? (i - 1 + n) % n : e.key === "Home" ? 0 : e.key === "End" ? n - 1 : -1;
    if (to < 0) return;
    e.preventDefault();
    onChange(tabs[to].id);
    refs.current[to]?.focus();
  };
  const active = tabs.find((t) => t.id === value);
  return (
    <div className="v-tabs">
      <div role="tablist" aria-label={label} className="v-tablist">
        {tabs.map((t, i) => (
          <button
            key={t.id}
            ref={(el) => { refs.current[i] = el; }}
            type="button"
            role="tab"
            id={`${base}-tab-${t.id}`}
            aria-selected={t.id === value}
            aria-controls={`${base}-panel-${t.id}`}
            tabIndex={t.id === value ? 0 : -1}
            className="v-tab"
            onClick={() => onChange(t.id)}
            onKeyDown={(e) => move(e, i)}
          >
            {t.label}
          </button>
        ))}
      </div>
      {active && (
        <div role="tabpanel" id={`${base}-panel-${active.id}`} aria-labelledby={`${base}-tab-${active.id}`} tabIndex={0} className="v-tabpanel">
          {active.panel}
        </div>
      )}
    </div>
  );
}
