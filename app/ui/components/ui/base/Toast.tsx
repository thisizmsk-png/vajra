import { useCallback, useEffect, useRef, useState } from "react";
import { S } from "../../../strings";
import { cn } from "../cn";
import { Button } from "./Button";
import { StatusDot } from "./StatusDot";

export type ToastItem = { id: number; kind: "info" | "success" | "danger"; text: string };

const SHOWN_MS = 5000;

function ToastView({ item, onDismiss }: { item: ToastItem; onDismiss: (id: number) => void }) {
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const start = useCallback(() => {
    timer.current = setTimeout(() => onDismiss(item.id), SHOWN_MS);
  }, [item.id, onDismiss]);
  const pause = () => clearTimeout(timer.current);
  useEffect(() => {
    start();
    return pause;
  }, [start]);
  const dot = item.kind === "danger" ? "failed" : item.kind === "success" ? "done" : "ready";
  return (
    <div className={cn("v-toast", `v-toast--${item.kind}`)} onMouseEnter={pause} onMouseLeave={start} onFocus={pause} onBlur={start}>
      <StatusDot status={dot} label="" />
      <span>{item.text}</span>
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
  const push = useCallback((kind: ToastItem["kind"], text: string) => setToasts((t) => [...t, { id: next.current++, kind, text }]), []);
  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  return { toasts, push, dismiss };
}
