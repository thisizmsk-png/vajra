// The message box docked under a conversation: the Director-General's, the Lead's and each worker's. It owns its draft
// and takes only stable props, so a streaming transcript re-rendering its parent never re-renders it, and a keystroke
// re-renders nothing else.
import { memo, useState } from "react";
import type { KeyboardEvent, ReactNode, RefObject } from "react";
import { Button, Kbd, ariaShortcut } from "../../../components/ui/base";
import { S } from "../../../strings";
import "./dg.css";

export type ComposerProps = {
  /** The textarea's id; the error line takes `<id>-err`. */
  id: string;
  label: string;
  placeholder: string;
  field?: RefObject<HTMLTextAreaElement>;
  available: boolean;
  canAsk: boolean;
  cancellable?: boolean;
  sending: boolean;
  askErr: string;
  /** Why the box is read only, shown in place of the shortcut hint. */
  offReason?: string;
  sendLabel?: string;
  /** Resolves to whether the text was taken; false puts it back. */
  onSend: (text: string) => Promise<boolean>;
  onCancel?: () => void;
  /** A second way to send the same draft (a worker's change of direction). */
  alt?: { label: string; onSend: (text: string) => Promise<boolean> };
  footer?: ReactNode;
};

export const Composer = memo(function Composer({ id, label, placeholder, field, available, canAsk, cancellable, sending, askErr, offReason, sendLabel, onSend, onCancel, alt }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const submit = async (how: (text: string) => Promise<boolean> = onSend) => {
    const text = draft;
    if (!text.trim() || !canAsk) return;
    setDraft("");
    if (!(await how(text))) setDraft((now) => now || text);
  };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void submit(); }
  };
  const err = `${id}-err`;
  const why = `${id}-why`;
  const described = askErr ? err : !available && offReason ? why : undefined;
  return (
    <form className="v-dgv__composer" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
      <div className="v-dgv__box">
        <label className="v-sr" htmlFor={id}>{label}</label>
        <textarea id={id} ref={field} className="v-dgv__field" rows={2} value={draft} placeholder={placeholder}
          onChange={(e) => setDraft(e.target.value)} onKeyDown={onKey} readOnly={!available}
          aria-disabled={!available || undefined} aria-describedby={described}
          aria-keyshortcuts={ariaShortcut(["⌘", "↵"])} />
        <div className="v-dgv__bar">
          {askErr ? <span id={err} className="v-dgv__err" role="alert">{askErr}</span>
            : !available && offReason ? <span id={why} className="v-dgv__off">{offReason}</span>
            : <span className="v-dgv__hint"><Kbd keys={["⌘", "↵"]} decorative /></span>}
          <span className="v-dgv__sends">
            {alt && !cancellable && <Button size="sm" variant="ghost" disabled={!canAsk || !draft.trim()} onClick={() => void submit(alt.onSend)}>{alt.label}</Button>}
            {cancellable
              ? <Button size="sm" variant="secondary" onClick={onCancel}>{S.dg.cancel}</Button>
              : <Button size="sm" variant="primary" type="submit" disabled={!canAsk || !draft.trim()} busy={sending}>{sendLabel ?? S.dg.send}</Button>}
          </span>
        </div>
      </div>
    </form>
  );
});
