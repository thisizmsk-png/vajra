// A worker's provider, wherever the worker appears: icon, "Model · provider" (never a bare model), the billing chip,
// "Ran on X (asked for Y)" when the harness ran something else, and the provider-error state. All from `effective`.
import { Badge } from "../../components/ui/base";
import { S } from "../../strings";
import { errorText, ranOn, workerLine } from "../workerProvider";
import type { WorkerProvider } from "../workerProvider";
import { ProviderIcon } from "./ProviderIcon";
import "./providers.css";

const W = S.workerProvider;
export const iconOf = (p: WorkerProvider) => <ProviderIcon family={p.family} via={p.family !== p.account ? p.account : undefined} />;

/** `compact`: one line (icon, model and provider) for a list row; the billing chip and the notes follow when there is room. `live`: the error is announced. */
export function ProviderLine({ p, compact, live }: { p: WorkerProvider; compact?: boolean; live?: boolean }) {
  const ran = ranOn(p);
  return (
    <span className={compact ? "v-wp v-wp--compact" : "v-wp"}>
      <span className="v-wp__main">
        {iconOf(p)}
        <span className="v-wp__line" data-model-line>{workerLine(p)}</span>
        {!compact && <Badge>{W.billing[p.billing] ?? W.billing.unknown}</Badge>}
      </span>
      {ran && <span className="v-wp__ran">{ran}</span>}
      {p.error && <span className={p.error.action === "retrying" ? "v-wp__err" : "v-wp__err v-wp__err--you"} role={live ? "status" : undefined}>{errorText(p.error)}</span>}
    </span>
  );
}
