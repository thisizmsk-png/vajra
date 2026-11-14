import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Section, Sheet, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, errorText, newActionId, onChanged } from "../../ipc";
import type { EvidenceApproval as Approval } from "../../ipc";
import { useCurrentProject } from "../../projects/current";
import { hidesText } from "./hidesText";

const L = S.run.approval;

type Shown = Approval & { sha256: string; ask: NonNullable<Approval["ask"]> };
const isPending = (a: Approval | null): a is Shown => !!a && a.state === "pending" && !!a.sha256 && !!a.ask;

/**
 * N20: the person sees the exact ordered check commands of a `guarded_by_approval` run, with what the approval is
 * bound to, and approves or declines the whole list. The answer names the sha of the list on screen, so the core
 * refuses it if the list it holds has changed. Only a click here answers: nothing opens or answers it for the person.
 */
export function EvidenceApproval({ runId }: { runId: string }) {
  const { project } = useCurrentProject();
  const [latest, setLatest] = useState<Approval | null>(null);
  const [err, setErr] = useState("");
  /** The list the dialog shows, fixed when it opened: a newer list never replaces it without the person asking. */
  const [shown, setShown] = useState<Shown | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const [answerErr, setAnswerErr] = useState("");
  const seq = useRef(0);
  const statusRef = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    const mine = ++seq.current;
    api.evidenceApproval(runId).then(
      (a) => { if (mine === seq.current) { setLatest(a); setErr(""); } },
      (e) => { if (mine === seq.current) setErr(errorText(e)); },
    );
  }, [runId]);
  useEffect(() => {
    setLatest(null); setErr(""); setShown(null); setNote(""); setAnswerErr("");
    load();
    const off = onChanged(load);
    return () => { seq.current++; off(); };
  }, [load]);

  // The core now holds a different pending list than the one on screen (PR 247 review: only then is "load the new
  // list" something the person can do). No pending list at all closes the dialog; the Section says why.
  const changed = !!shown && isPending(latest) && latest.sha256 !== shown.sha256;
  useEffect(() => {
    if (shown && latest && !isPending(latest) && !busy) {
      setShown(null);
      requestAnimationFrame(() => statusRef.current?.focus());
    }
  }, [shown, latest, busy]);
  // The same fields the core checks: every command, the seat and the account (all shown in the dialog).
  const hidden = !!shown && [...shown.ask.commands, shown.ask.seat, shown.ask.account].some(hidesText);
  const blockedWhy = changed ? L.changed : hidden ? L.hidden : "";

  const open = () => {
    if (!isPending(latest)) return;
    setNote(""); setAnswerErr("");
    setShown(latest);
  };
  const close = () => { if (!busy) setShown(null); };
  const loadNew = () => {
    setAnswerErr("");
    setShown(isPending(latest) ? latest : null);
  };
  const done = (text: string) => {
    setShown(null);
    setNote(text);
    load();
    // The button that opened the dialog is gone once the list is answered: put focus on the outcome instead.
    requestAnimationFrame(() => statusRef.current?.focus());
  };

  const answer = (approve: boolean) => {
    if (!shown || busy || (approve && blockedWhy) || (!approve && changed)) return;
    const target = shown;
    setBusy(true); setAnswerErr("");
    const env = { action_id: newActionId(), project_id: project?.id ?? "global", expect_generation: null };
    api.actAnswerEvidenceApproval(env, target.run_id, target.sha256, approve).then(
      (r) => {
        if (r.result) return done(approve ? L.approvedNote : L.declinedNote);
        const code = r.error?.error ?? r.receipt?.reason;
        if (code === "list_changed") { setAnswerErr(L.listChanged); load(); return; }
        if (code === "expired") return done(L.state.expired);
        if (code === "record_changed") return done(L.state.void);
        if (code === "not_pending" || code === "not_found") return done(L.notPending);
        setAnswerErr(L.notSaved(r.error?.detail ?? r.receipt?.detail ?? S.run.failed));
      },
      (e) => setAnswerErr(L.notSaved(errorText(e))),
    ).finally(() => setBusy(false));
  };

  const stateText = latest && latest.state !== "pending" && latest.state !== "none" && latest.state !== "consumed" ? L.state[latest.state] : "";
  if (!err && !note && !isPending(latest) && !stateText) return null;

  return (
    <Section title={L.title}>
      {err && (
        <div role="alert" className="v-approval__line">
          <Text variant="body-sm">{L.failed} {err}</Text>
          <Button size="sm" variant="ghost" onClick={load}>{L.retry}</Button>
        </div>
      )}
      {isPending(latest) && (
        <div className="v-approval__line">
          <Text as="span" variant="body-sm">{L.waiting}</Text>
          <Button size="sm" variant="primary" onClick={open}>{L.review}</Button>
        </div>
      )}
      <div ref={statusRef} tabIndex={-1} role="status" className="v-approval__status">
        {note ? <Text variant="body-sm">{note}</Text> : stateText ? <Text variant="body-sm">{stateText}</Text> : null}
      </div>
      <Sheet open={!!shown} onClose={close} title={L.dialogTitle} className="v-approval">
        {shown && (
          <>
            <Text variant="body-sm" muted>{L.intro}</Text>
            <dl className="v-approval__facts">
              <dt>{L.run}</dt><dd><Text as="span" variant="mono">{shown.ask.run_id}</Text></dd>
              <dt>{L.round}</dt><dd>{shown.ask.round}</dd>
              <dt>{L.commit}</dt><dd><Text as="span" variant="mono" className="v-approval__wrap">{shown.ask.commit}</Text></dd>
              <dt>{L.seat}</dt><dd>{shown.ask.seat}</dd>
              <dt>{L.account}</dt><dd>{shown.ask.account}</dd>
              {/* The runner's own setting, bound into the list hash: what the person vouches these commands may reach. */}
              <dt>{L.network}</dt><dd>{shown.ask.network ? L.networkOn : L.networkOff}</dd>
              <dt>{L.listSha}</dt><dd><Text as="span" variant="mono" className="v-approval__wrap">{shown.sha256}</Text></dd>
            </dl>
            <h3 id="v-approval-list" className="v-text v-text--label">{L.list(shown.ask.commands.length)}</h3>
            {/* Focusable so the list scrolls from the keyboard. Each command is a text node, shown whole and wrapped. */}
            <ol className="v-approval__list" aria-labelledby="v-approval-list" tabIndex={0}>
              {shown.ask.commands.map((c, i) => (
                <li key={i}><code>{c}</code></li>
              ))}
            </ol>
            <Text variant="body-sm" muted>{L.scope}</Text>
            <Text variant="body-sm" muted>{L.declineNote}</Text>
            {blockedWhy && (
              <div id="v-approval-why" role="alert" className="v-approval__line">
                <Text variant="body-sm">{blockedWhy}</Text>
                {changed && isPending(latest) && <Button size="sm" onClick={loadNew}>{L.loadNew}</Button>}
              </div>
            )}
            {answerErr && !blockedWhy && <div role="alert"><Text variant="body-sm">{answerErr}</Text></div>}
            <div className="v-dialog__actions">
              <Button data-autofocus variant="ghost" disabled={busy} onClick={close}>{L.notNow}</Button>
              <Button disabled={changed} busy={busy} onClick={() => answer(false)} aria-describedby={changed ? "v-approval-why" : undefined}>{L.decline}</Button>
              <Button variant="primary" disabled={!!blockedWhy} busy={busy} onClick={() => answer(true)} aria-describedby={blockedWhy ? "v-approval-why" : undefined}>
                {busy ? L.answering : L.approve}
              </Button>
            </div>
          </>
        )}
      </Sheet>
    </Section>
  );
}
