import { useEffect, useState } from "react";
import { cn } from "../cn";

/** Placeholder for content that is loading. Rendered only after `delay` ms (300 by default), so fast loads never flash. */
export function Skeleton({ variant = "row", delay = 300 }: { variant?: "row" | "block"; delay?: number }) {
  const [shown, setShown] = useState(delay <= 0);
  useEffect(() => {
    if (delay <= 0) return;
    const t = setTimeout(() => setShown(true), delay);
    return () => clearTimeout(t);
  }, [delay]);
  return shown ? <span className={cn("v-skeleton", variant === "block" && "v-skeleton--block")} aria-busy="true" /> : null;
}
