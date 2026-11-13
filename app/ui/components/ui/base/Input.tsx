import { useId } from "react";
import type { ReactNode } from "react";

/** `trailing` sits on the control's row, after it (a "Choose…" button beside a path, for example). */
type Common = { label: string; error?: string; hint?: string; id?: string; trailing?: ReactNode };
type Props =
  | (Common & { type?: "text" | "number" | "password"; value: string; onChange: (v: string) => void; placeholder?: string })
  | (Common & { type: "textarea"; value: string; onChange: (v: string) => void; rows?: number; placeholder?: string })
  | (Common & { type: "select"; value: string; onChange: (v: string) => void; options: Array<{ value: string; label: string }> });

/** Label, control, hint and error wired together: `<label>`, aria-invalid, aria-describedby. */
export function Input(props: Props) {
  const auto = useId();
  const id = props.id ?? auto;
  const hintId = props.hint ? `${id}-hint` : undefined;
  const errId = props.error ? `${id}-err` : undefined;
  const common = {
    id,
    className: "v-input",
    "aria-invalid": props.error ? true : undefined,
    "aria-describedby": [hintId, errId].filter(Boolean).join(" ") || undefined,
  } as const;
  let control: ReactNode;
  if (props.type === "textarea") {
    control = <textarea {...common} rows={props.rows} placeholder={props.placeholder} value={props.value} onChange={(e) => props.onChange(e.target.value)} />;
  } else if (props.type === "select") {
    control = (
      <select {...common} value={props.value} onChange={(e) => props.onChange(e.target.value)}>
        {props.options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    );
  } else {
    control = (
      <input {...common} type={props.type ?? "text"} autoComplete={props.type === "password" ? "off" : undefined} spellCheck={props.type === "password" ? false : undefined} value={props.value} placeholder={props.placeholder} onChange={(e) => props.onChange(e.target.value)} />
    );
  }
  return (
    <div className="v-field">
      <label className="v-field__label" htmlFor={id}>
        {props.label}
      </label>
      {props.trailing ? <div className="v-field__row">{control}{props.trailing}</div> : control}
      {props.hint && (
        <span id={hintId} className="v-field__hint">
          {props.hint}
        </span>
      )}
      {props.error && (
        <span id={errId} className="v-field__error" role="alert">
          {props.error}
        </span>
      )}
    </div>
  );
}
