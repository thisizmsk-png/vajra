// One setup check as a row: number, title, a status (shape and word, never colour alone), a one-line summary and a body that
// opens under it. The welcome lists these; a later item shows a failed check as the same row on the Start screen, so the row
// knows nothing about the welcome. `CheckList` adds the arrow-key movement between row heads.
import { useId } from "react";
import type { KeyboardEvent, ReactNode, Ref } from "react";
import { Icon } from "../../components/ui/base";
import { StatusDot } from "../../components/ui/base";
import type { StatusKey } from "../../strings";
import { S } from "../../strings";
import "./welcome.css";

export type CheckKind = "pass" | "warn" | "todo" | "fail";
/** The kit's shape for each kind: check, triangle, ring, cross. */
const DOT: Record<CheckKind, StatusKey> = { pass: "pass", warn: "needs_approval", todo: "pending", fail: "fail" };

export interface CheckRowProps {
  /** The row's place in the list; leave out where the row stands alone. */
  n?: number;
  title: string;
  kind: CheckKind;
  /** The status word beside the shape. Defaults to the shared word for the kind. */
  word?: string;
  summary: string;
  open: boolean;
  onToggle: () => void;
  headRef?: Ref<HTMLButtonElement>;
  children?: ReactNode;
}

export function CheckRow({ n, title, kind, word, summary, open, onToggle, headRef, children }: CheckRowProps) {
  const id = useId();
  return (
    <li className="v-chk" data-kind={kind} data-open={open || undefined}>
      <h3 className="v-chk__h">
        <button type="button" ref={headRef} className="v-chk__head" id={`${id}-head`} aria-expanded={open} aria-controls={`${id}-body`} onClick={onToggle}>
          {n !== undefined && <span className="v-chk__n" aria-hidden="true">{n}</span>}
          <span className="v-chk__t">{title}</span>
          <span className="v-chk__st"><StatusDot status={DOT[kind]} label={word ?? S.welcome.word[kind]} /></span>
          <span className="v-chk__sum">{summary}</span>
          <span className="v-chk__chev" aria-hidden="true"><Icon name="chevron" size={12} /></span>
        </button>
      </h3>
      {open && (
        <div className="v-chk__body" id={`${id}-body`} role="region" aria-labelledby={`${id}-head`}>
          {children}
        </div>
      )}
    </li>
  );
}

/** The list of rows. Up and Down move between their heads, Home and End go to the first and last; Return and Space open one. */
export function CheckList({ label, children }: { label: string; children: ReactNode }) {
  const onKeyDown = (e: KeyboardEvent<HTMLOListElement>) => {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const target = e.target as HTMLElement;
    if (!target.classList.contains("v-chk__head")) return;
    const heads = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(".v-chk__head"));
    const i = heads.indexOf(target);
    const next = e.key === "ArrowDown" ? heads[i + 1] : e.key === "ArrowUp" ? heads[i - 1] : e.key === "Home" ? heads[0] : e.key === "End" ? heads[heads.length - 1] : undefined;
    if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      next?.focus();
    }
  };
  return <ol className="v-chk-list" aria-label={label} onKeyDown={onKeyDown}>{children}</ol>;
}
