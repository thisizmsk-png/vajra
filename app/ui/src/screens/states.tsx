import { useState, type ReactNode } from "react";
import { Button, Skeleton, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { errorText, isIpcError } from "../ipc";

/**
 * What to say about a failed call. The core answers with a code and a detail, shown small under a plain title; a
 * rejection that is not the core's answer means the core did not answer at all, which gets its own plain words.
 */
export const problemDetail = (e: unknown): string => (isIpcError(e) ? errorText(e) : S.states.noAnswer);

const clock = (d: Date) => d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

/**
 * A screen or section could not load: a plain title, the detail, and Try again when it can be retried. A retry that
 * fails the same way would leave the screen unchanged, so the box says when it last tried.
 */
export function Problem({ title, detail, retry }: { title: string; detail?: string; retry?: () => void }) {
  const [tried, setTried] = useState<Date | null>(null);
  return (
    <div role="alert" className="v-problem">
      <Text variant="label">{title}</Text>
      {detail && <Text variant="body-sm" muted className="v-problem__detail">{detail}</Text>}
      {retry && <Button size="sm" onClick={() => { setTried(new Date()); retry(); }}>{S.states.retry}</Button>}
      {retry && <span role="status" className="v-checked">{tried ? S.states.triedAt(clock(tried)) : ""}</span>}
    </div>
  );
}

/** Nothing here yet: what this place is for, and the one real next step. Never a blank area. */
export function Empty({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="v-empty">
      <Text as="h2" variant="headline-md">{title}</Text>
      {children && <Text variant="body" muted>{children}</Text>}
      {action && <div className="v-empty__act">{action}</div>}
    </div>
  );
}

/** Rows that are on their way: the shape of the list, so the layout does not jump when it lands. */
export function LoadingRows({ rows = 3, label }: { rows?: number; label: string }) {
  return (
    <div className="v-loading" role="status" aria-label={label}>
      {Array.from({ length: rows }, (_, i) => <Skeleton key={i} />)}
    </div>
  );
}

/**
 * When a check last finished. A re-check that finds nothing new would otherwise look like a dead click, so every
 * finished check updates this line and a screen reader hears it once. The region exists before the first answer.
 */
export function CheckedAt({ at }: { at: Date | null }) {
  return <span role="status" className="v-checked">{at ? S.states.checkedAt(clock(at)) : ""}</span>;
}
