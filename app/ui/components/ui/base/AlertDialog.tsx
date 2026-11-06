import type { ReactNode } from "react";
import { S } from "../../../strings";
import { Button } from "./Button";
import { Sheet } from "./Sheet";

/**
 * Confirm or destructive confirmation. The safe button (cancel) holds the initial focus, so Enter never
 * confirms a stop by accident.
 */
export function AlertDialog({
  open,
  title,
  confirmLabel,
  cancelLabel = S.cancel,
  destructive,
  onConfirm,
  onCancel,
  children,
}: {
  open: boolean;
  title: string;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  children?: ReactNode;
}) {
  return (
    <Sheet open={open} onClose={onCancel} title={title} role="alertdialog">
      {children}
      <div className="v-dialog__actions">
        <Button data-autofocus variant="secondary" onClick={onCancel}>
          {cancelLabel}
        </Button>
        <Button variant={destructive ? "danger" : "primary"} onClick={onConfirm}>
          {confirmLabel}
        </Button>
      </div>
    </Sheet>
  );
}
