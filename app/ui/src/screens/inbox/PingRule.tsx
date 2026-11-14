// The ping rule, where the pings land: one line that says the rule in words, a Change button that opens it inline, and
// the pings since the app opened, each with why it pinged (or why it did not). The rule is a global preference saved
// through the receipt-checked preferences action; it decides only when you are pinged, never what happens to an item.
import { useState } from "react";
import { Button, Switch } from "../../../components/ui/base";
import { S } from "../../../strings";
import { usePreferenceScope } from "../../preferences";
import { DEFAULT_PING, PING_BATCH_MINUTES, PING_PER_HOUR } from "../../preferenceContract";
import type { PingRule as Rule } from "../../preferenceContract";
import { PING_GROUPS } from "../../projects/ping";
import { usePingLog } from "../../projects/PingHost";
import "./pingRule.css";

const R = S.inbox.rule;
const time = (at: number) => new Date(at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
/** A stored value the list does not offer stays visible, never silently swapped. */
const withValue = (list: readonly number[], v: number) => (list.includes(v) ? [...list] : [...list, v].sort((a, b) => a - b));

export function PingRule() {
  // Global scope, always: the core refuses a project ping rule.
  const prefs = usePreferenceScope(null);
  const rule: Rule = prefs.view?.global.ping ?? DEFAULT_PING;
  const [draft, setDraft] = useState<Rule | null>(null);
  const [message, setMessage] = useState("");
  const log = usePingLog();
  const on = PING_GROUPS.filter((g) => rule[g]);
  const editing = draft !== null;

  const save = async () => {
    if (!draft) return;
    setMessage("");
    if (await prefs.save({ ping: draft })) { setDraft(null); setMessage(R.saved); }
  };

  return (
    <section className="v-ping" aria-label={R.title}>
      <div className="v-ping__line">
        <h2 className="v-ping__h">{R.title}</h2>
        <p className="v-ping__sum">{prefs.view ? R.summary(rule.per_hour, rule.batch_minutes, on) : prefs.error ? R.unavailable : ""}</p>
        {!editing && <Button size="sm" variant="ghost" disabled={!prefs.view} onClick={() => { setMessage(""); setDraft(rule); }}>{R.edit}</Button>}
      </div>
      {message && <p role="status" className="v-ping__note">{message}</p>}
      {editing && (
        <form className="v-ping__form" onSubmit={(e) => { e.preventDefault(); void save(); }}>
          <label className="v-ping__row"><span>{R.perHour}</span>
            <select value={draft.per_hour} onChange={(e) => setDraft({ ...draft, per_hour: Number(e.target.value) })}>
              {withValue(PING_PER_HOUR, draft.per_hour).map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
          </label>
          <label className="v-ping__row"><span>{R.batch}</span>
            <select value={draft.batch_minutes} onChange={(e) => setDraft({ ...draft, batch_minutes: Number(e.target.value) })}>
              {withValue(PING_BATCH_MINUTES, draft.batch_minutes).map((m) => <option key={m} value={m}>{R.batchOption(m)}</option>)}
            </select>
          </label>
          <fieldset className="v-ping__kinds">
            <legend>{R.kinds}</legend>
            {PING_GROUPS.map((g) => (
              <div className="v-ping__row" key={g}>
                <span>{S.ping.groupLabels[g]}<small>{S.ping.groupHints[g]}</small></span>
                <Switch label={S.ping.groupLabels[g]} checked={draft[g]} onChange={(v) => setDraft({ ...draft, [g]: v })} />
              </div>
            ))}
          </fieldset>
          <p className="v-ping__note">{R.note}</p>
          {prefs.error && <p role="alert" className="v-ping__err">{prefs.error}</p>}
          <div className="v-ping__acts">
            <Button size="sm" variant="ghost" type="button" onClick={() => setDraft(null)}>{S.cancel}</Button>
            <Button size="sm" variant="primary" type="submit" busy={prefs.busy}>{R.save}</Button>
          </div>
        </form>
      )}
      <details className="v-ping__log">
        <summary>{S.inbox.recent}{log.length > 0 && <span className="v-num"> {log.length}</span>}</summary>
        {log.length === 0 ? <p className="v-ping__note">{S.inbox.recentEmpty}</p> : (
          <ol aria-label={S.inbox.recent}>
            {log.map((e, i) => (
              <li key={`${e.at}-${i}`} className={e.pinged ? "v-ping__e" : "v-ping__e v-ping__e--quiet"}>
                <span className="v-ping__when"><time dateTime={new Date(e.at).toISOString()}>{time(e.at)}</time> · {e.pinged ? S.inbox.pinged : S.inbox.quiet}</span>
                <span>{e.text}</span>
                <span className="v-ping__why">{e.why}</span>
              </li>
            ))}
          </ol>
        )}
      </details>
    </section>
  );
}
