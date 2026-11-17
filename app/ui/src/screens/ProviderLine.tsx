// A worker's provider, wherever the worker appears: icon, "Model via provider" (never a bare model), the billing chip,
// "Ran on X (asked for Y)" when the harness ran something else, and the provider-error state. All from `effective`.
import { Badge, Icon } from "../../components/ui/base";
import { S } from "../../strings";
import { diffLabel, diffText, errorText, ranOn, workerLine } from "../workerProvider";
import type { WorkerDiff, WorkerProvider } from "../workerProvider";
import { ProviderIcon } from "./ProviderIcon";
import "./providers.css";

const W = S.workerProvider;
export const iconOf = (p: WorkerProvider) => <ProviderIcon family={p.family} model={p.model} via={p.family !== p.account ? p.account : undefined} />;

/** `+N −M`, or `≥ +N lines` when some new files were not counted. The label says what the short form means. */
export function DiffCount({ d }: { d: WorkerDiff }) {
  return <span className="v-wp__diff" data-diff-count aria-label={diffLabel(d)} title={diffLabel(d)}>{diffText(d)}</span>;
}

/**
 * For a dense row (the project column): the provider's icon only, with "Model via provider" in its tooltip and for
 * screen readers. A worker that ran on another model, or has a provider error, gets one warning glyph instead of a
 * wrapped sentence; the full words are in its tooltip, read out with the row, and shown in full in the worker's pane.
 */
export function ProviderMark({ p }: { p: WorkerProvider }) {
  const ran = ranOn(p);
  const note = [ran, p.error ? errorText(p.error) : null].filter(Boolean).join(" ");
  const you = !!ran || (!!p.error && p.error.action !== "retrying");
  return (
    <span className="v-wm" title={workerLine(p)}>
      {iconOf(p)}
      <span className="v-sr">{workerLine(p)}</span>
      {note && (
        <span className={you ? "v-wm__warn v-wm__warn--you" : "v-wm__warn"} title={note} data-provider-note>
          <Icon name="alert" size={12} /><span className="v-sr">{note}</span>
        </span>
      )}
    </span>
  );
}

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
