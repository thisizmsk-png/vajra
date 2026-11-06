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
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
    >
      <h2 id={titleId} className="v-text v-text--headline-md">
        {title}
      </h2>
      {children}
    </dialog>
  );
}
