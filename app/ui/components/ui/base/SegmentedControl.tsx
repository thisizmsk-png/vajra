import { useId } from "react";

/** 2 to 4 options as native radios in a fieldset: arrow keys, one tab stop and announcements come from the browser. */
export function SegmentedControl<V extends string>({ label, value, options, onChange }: { label: string; value: V; options: ReadonlyArray<{ value: V; label: string }>; onChange: (v: V) => void }) {
  const name = useId();
  return (
    <fieldset className="v-seg">
      <legend className="v-sr">{label}</legend>
      {options.map((o) => (
        <label key={o.value}>
          <input type="radio" name={name} value={o.value} checked={o.value === value} onChange={() => onChange(o.value)} />
          {o.label}
        </label>
      ))}
    </fieldset>
  );
}
