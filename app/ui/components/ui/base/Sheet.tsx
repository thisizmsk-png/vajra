import { useEffect, useId, useRef } from "react";
import type { ReactNode } from "react";
import { cn } from "../cn";

/**
 * A modal layer on the native <dialog>: the browser traps focus, closes on Esc and gives focus back to the
 * element that opened it. `open` is controlled; `onClose` fires for Esc and for the backdrop.
 */
export function Sheet({
  open,
  onClose,
  title,
  side = "center",
  role,
  className,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  side?: "center" | "right";
  role?: "dialog" | "alertdialog";
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const pressedBackdrop = useRef(false);
  const titleId = useId();
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      d.showModal();
      // The native dialog focuses its first control; a marked "safe" control wins (AlertDialog's cancel).
      d.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    }
    if (!open && d.open) d.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      role={role}
      aria-labelledby={titleId}
      className={cn("v-dialog", side === "right" && "v-dialog--right", className)}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      // A close the parent did not ask for (form method=dialog, programmatic close) still reaches the parent.
      onClose={() => open && onClose()}
      // Press and release both on the backdrop: a text selection dragged out of the dialog must not close it.
      onMouseDown={(e) => {
        pressedBackdrop.current = e.target === ref.current;
      }}
      onClick={(e) => {
        if (e.target === ref.current && pressedBackdrop.current) onClose();
      }}
    >
      <h2 id={titleId} className="v-text v-text--headline-md">
        {title}
      </h2>
      {children}
    </dialog>
  );
}
