import { useId } from "react";
import { Badge } from "../base/Badge";
import { Button } from "../base/Button";

export type ActionState = "queued" | "delivered" | "acknowledged" | "failed";
const WORDS: Record<ActionState, string> = { queued: "Queued", delivered: "Delivered", acknowledged: "Acknowledged", failed: "Failed" };

/**
 * An orchestrator or human control action on a worker (message, redirect, stop request). The state is in a live
 * region so a change is announced; the worker and the audit line are links the host wires (callbacks, not URLs).
 */
export function ActionCard({
  kind,
  worker,
  by,
  text,
  state,
  time,
  onOpenWorker,
  onOpenAudit,
}: {
  kind: "message" | "redirect" | "stop";
  worker: string;
  by: "Lead" | "You";
  text?: string;
  state: ActionState;
  time: string;
  onOpenWorker?: () => void;
  onOpenAudit?: () => void;
}) {
  const id = useId();
  const verb = kind === "message" ? "Message to" : kind === "redirect" ? "Redirect for" : "Stop request for";
  return (
    <article className="v-card" aria-labelledby={id}>
      <h3 id={id} className="v-text v-text--label">{verb} {worker}</h3>
      <p className="v-text v-text--body-sm">{by}, <span className="v-num">{time}</span></p>
      {text && <p className="v-text v-text--body">{text}</p>}
      <div className="v-action-foot">
        <span role="status" aria-live="polite"><Badge status={state === "failed" ? "danger" : state === "acknowledged" ? "success" : "info"}>{WORDS[state]}</Badge></span>
        {onOpenWorker && <Button size="sm" variant="ghost" onClick={onOpenWorker}>Open worker</Button>}
        {onOpenAudit && <Button size="sm" variant="ghost" onClick={onOpenAudit}>Open audit line</Button>}
      </div>
    </article>
  );
}
