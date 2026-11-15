import { useCallback, useEffect, useRef, useState } from "react";
import { S } from "../../../strings";
import type { StatusKey } from "../../../strings";
import { cn } from "../cn";
import { Button } from "./Button";
import { StatusDot } from "./StatusDot";

/** `detail` is a second, quieter line: for a ping, why it pinged. `label` replaces the dot's status word. */
export type ToastItem = { id: number; kind: "info" | "success" | "danger"; text: string; detail?: string; label?: string };

const SHOWN_MS = 5000;
const DOT: Record<ToastItem["kind"], StatusKey> = { info: "ready", success: "done", danger: "failed" };

function ToastView({ item, onDismiss }: { item: ToastItem; onDismiss: (id: number) => void }) {
  const [paused, setPaused] = useState(false);
  // Info and success go after 5 s unless hovered or focused. An error stays until the user dismisses it (WCAG 2.2.1).
  useEffect(() => {
    if (paused || item.kind === "danger") return;
    const t = setTimeout(() => onDismiss(item.id), SHOWN_MS);
    return () => clearTimeout(t);
  }, [paused, item.id, item.kind, onDismiss]);
  return (
    <div
      className={cn("v-toast", `v-toast--${item.kind}`)}
      role={item.kind === "danger" ? "alert" : undefined}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocus={() => setPaused(true)}
      onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && setPaused(false)}
    >
      <StatusDot status={DOT[item.kind]} label={item.label} />
      {item.detail ? (
        <span className="v-toast__body"><span>{item.text}</span><span className="v-toast__detail">{item.detail}</span></span>
      ) : <span>{item.text}</span>}
      <Button size="sm" variant="ghost" onClick={() => onDismiss(item.id)}>
        {S.dismiss}
      </Button>
    </div>
  );
}

/** The live region. A toast is never the only record of an action: the audit line and the ledger are. */
export function ToastRegion({ toasts, onDismiss }: { toasts: ToastItem[]; onDismiss: (id: number) => void }) {
  return (
    <div className="v-toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <ToastView key={t.id} item={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

export function useToasts() {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const next = useRef(1);
  const push = useCallback((kind: ToastItem["kind"], text: string, detail?: string, label?: string) =>
    setToasts((t) => [...t, { id: next.current++, kind, text, detail, label }]), []);
  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  return { toasts, push, dismiss };
}
