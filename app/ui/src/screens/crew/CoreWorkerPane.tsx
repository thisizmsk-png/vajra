import { useId, useState } from "react";
import { AlertDialog, Button, Input, Section, StatusDot, Text } from "../../../components/ui/base";
import type { StatusKey } from "../../../strings";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { CrewNodeDto } from "../../ipc";
import { CONTROL, INVISIBLE } from "../../projectContract";
import { DiffCount, ProviderLine } from "../ProviderLine";
import { parseWorkerDiff, parseWorkerProvider } from "../../workerProvider";

/** One control as the core's harness profile allows it: off with the core's reason, or on. */
export type CoreControl = { enabled: boolean; reason: string | null };
export type CoreControls = { message: CoreControl; redirect: CoreControl; stop: { enabled: boolean; note: string | null } };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const line = (v: unknown): string | null => (typeof v === "string" && v.trim() !== "" && [...v].length <= 300 && !CONTROL.test(v) && !INVISIBLE.test(v) ? v : null);
const one = (v: unknown): CoreControl | null => (isObj(v) && typeof v.enabled === "boolean" ? { enabled: v.enabled, reason: line(v.reason) } : null);

/** A `core:` node's `controls` (16.7); null when it is missing or malformed, and then every control is off. */
export function parseCoreControls(v: unknown): CoreControls | null {
  if (!isObj(v) || !isObj(v.stop) || typeof v.stop.enabled !== "boolean") return null;
  const message = one(v.message), redirect = one(v.redirect);
  if (!message || !redirect) return null;
  return { message, redirect, stop: { enabled: v.stop.enabled, note: line(v.stop.note) } };
}

const STATUS_KEYS = ["pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed", "unknown"] as const;
const C = S.crew.core;

/** A worker the core runs itself (`core:<ticket>`): its change set and its harness's Message, Redirect and Stop. Every
 * action names the full `core:` reference, which is what the core's control service routes on. */
export function CoreWorkerPane({ node, status }: { node: CrewNodeDto; status: string }) {
  const wp = parseWorkerProvider(node.provider);
  const wd = parseWorkerDiff(node.diff);
  const controls = parseCoreControls(node.controls);
  const [text, setText] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const why = useId();
  const shape = (STATUS_KEYS as readonly string[]).includes(node.status) ? (node.status as StatusKey) : "unknown";

  const send = (kind: "message" | "redirect") => {
    const c = controls?.[kind];
    if (!c?.enabled || !text.trim() || busy) return;
    setBusy(true);
    const call = kind === "message" ? api.controlMessage : api.controlRedirect;
    void call(node.id, text).then(
      () => { setText(""); setNote(C.sent); },
      (e) => setNote(C.notSent(errorText(e))),
    ).finally(() => setBusy(false));
  };
  const stop = () => {
    setConfirming(false);
    setBusy(true);
    void api.controlStop(node.id).then(() => setNote(C.stopped), (e) => setNote(C.notStopped(errorText(e)))).finally(() => setBusy(false));
  };
  // Message and Redirect share one reason when both are off for the same one (the profile's usual case).
  const off = controls ? [controls.message, controls.redirect].filter((c) => !c.enabled).map((c) => c.reason).filter((r): r is string => !!r) : [];
  const reasons = [...new Set(off)];

  return (
    <>
      <section className="v-crew__pane" aria-label={S.crew.header}>
        <div className="v-crew__panehead">
          <Text as="h2" variant="headline-md">{node.role ?? node.title}</Text>
          <StatusDot status={shape} label={status} />
          {wp && <ProviderLine p={wp} live />}
          {wd && <DiffCount d={wd} />}
        </div>
        <div className="v-crew__changes">
          <Section title={C.changes}>
            {wd && wd.files.length ? (
              <>
                <ul className="v-crew__files" aria-label={C.changes}>{wd.files.map((f) => <li key={f}><code>{f}</code></li>)}</ul>
                {wd.files_total > wd.files.length && <Text variant="body-sm" muted>{C.moreFiles(wd.files_total - wd.files.length)}</Text>}
              </>
            ) : <Text variant="body-sm" muted>{C.noDiff}</Text>}
          </Section>
        </div>
      </section>
      <aside className="v-crew__aside" aria-label={C.controls}>
        <Section title={C.controls}>
          {!controls ? <Text variant="body-sm" muted>{C.noControls}</Text> : (
            <div className="v-crew__controls">
              <Input type="textarea" label={C.text} rows={3} value={text} onChange={setText} />
              <div className="v-crew__controlrow">
                <Button size="sm" disabled={!controls.message.enabled || !text.trim()} busy={busy} aria-describedby={controls.message.enabled ? undefined : why} onClick={() => send("message")}>{C.message}</Button>
                <Button size="sm" disabled={!controls.redirect.enabled || !text.trim()} busy={busy} aria-describedby={controls.redirect.enabled ? undefined : why} onClick={() => send("redirect")}>{C.redirect}</Button>
                <Button size="sm" variant="danger" disabled={!controls.stop.enabled} busy={busy} onClick={() => setConfirming(true)}>{C.stop}</Button>
              </div>
              {reasons.length > 0 && <p id={why} className="v-crew__reason">{reasons.join(" ")}</p>}
              {controls.stop.note && <Text variant="body-sm" muted>{controls.stop.note}</Text>}
            </div>
          )}
          {note && <p className="v-crew__reason" role="status">{note}</p>}
        </Section>
        <AlertDialog open={confirming} title={C.stopTitle} confirmLabel={C.stop} destructive onConfirm={stop} onCancel={() => setConfirming(false)}>
          <p>{controls?.stop.note ?? ""}</p>
        </AlertDialog>
      </aside>
    </>
  );
}
