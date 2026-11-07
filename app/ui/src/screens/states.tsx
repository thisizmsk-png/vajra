import type { ReactNode } from "react";
import { Button, Skeleton, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { errorText, isIpcError } from "../ipc";

/**
 * What to say about a failed call. The core answers with a code and a detail, shown small under a plain title; a
 * rejection that is not the core's answer means the core did not answer at all, which gets its own plain words.
 */
export const problemDetail = (e: unknown): string => (isIpcError(e) ? errorText(e) : S.states.noAnswer);

/** A screen or section could not load: a plain title, the detail, and Try again when it can be retried. */
export function Problem({ title, detail, retry }: { title: string; detail?: string; retry?: () => void }) {
  return (
    <div role="alert" className="v-problem">
      <Text variant="label">{title}</Text>
      {detail && <Text variant="body-sm" muted className="v-problem__detail">{detail}</Text>}
      {retry && <Button size="sm" onClick={retry}>{S.states.retry}</Button>}
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
