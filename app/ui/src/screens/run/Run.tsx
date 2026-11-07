import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Section, Text } from "../../../components/ui/base";
import { VirtualList } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { EvidenceDetail, EvidenceRow } from "../../ipc";
import { useRoute } from "../../router";

const ORDER: Record<string, number> = { fail: 0, unverified: 1, checking: 2, not_run: 3, stale: 4, pass: 5 };
const BADGE: Record<string, "success" | "warning" | "danger" | "info" | undefined> = { pass: "success", fail: "danger", unverified: "danger", checking: "info", not_run: undefined, stale: "warning" };
const keyOf = (r: { task: string; criterion: string }) => `${r.task}\u0000${r.criterion}`;
const ROW_H = 36;
const LIST_H = 360;

const label = (s: string) => (S.run.status as Record<string, string>)[s] ?? S.run.status.not_run;

/** Evidence is the Run view's default (POS-1.1). Rows render through VirtualList (only the rows in view exist) and are plain data. */
export function Run() {
  const [route] = useRoute();
  const slug = route.params.slug ?? "";
  const [rows, setRows] = useState<EvidenceRow[] | null>(null);
  const [err, setErr] = useState("");
  /** Rows whose check has not run yet: the core checks a bounded amount per call, so ask again until none are left. */
  const [pending, setPending] = useState(0);
  /** The chosen criterion by identity (task + criterion), not list position: statuses change while polling and reorder the list. */
  const [sel, setSel] = useState<string | null>(null);
  const [detail, setDetail] = useState<EvidenceDetail | "failed" | null>(null);

  /** The slug the latest request was made for: a response for any other slug (the route changed while it was in flight) is dropped. */
  const current = useRef(slug);
  const load = useCallback(() => {
    if (!slug) return;
    current.current = slug;
    api.evidenceList(slug).then(
      (r) => { if (current.current !== slug) return; setRows(r.rows); setPending(r.pending ?? 0); setErr(""); },
      (e) => { if (current.current === slug) setErr(errorText(e)); },
    );
  }, [slug]);
  // A new campaign starts empty: the previous campaign's rows, pending count, error and selection are not shown under it.
  useEffect(() => { current.current = slug; setRows(null); setPending(0); setErr(""); setSel(null); }, [slug]);
  useEffect(load, [load]);
  useEffect(() => (slug ? onChanged(load) : undefined), [slug, load]);
  useEffect(() => {
    if (!pending) return;
    const t = setTimeout(load, 1500);
    return () => clearTimeout(t);
  }, [pending, rows, load]);

  // Failing first; ties keep the campaign's own order.
  const sorted = useMemo(
    () => (rows ?? []).map((r, i) => [r, i] as const).sort((a, b) => (ORDER[a[0].status] ?? 9) - (ORDER[b[0].status] ?? 9) || a[1] - b[1]).map((x) => x[0]),
    [rows],
  );
  const passing = sorted.filter((r) => r.status === "pass").length;

  const picked = sel === null ? null : sorted.find((r) => keyOf(r) === sel) ?? null;
  const pickedTask = picked?.task;
  // The recorded commands and their output come from the core one task at a time, only for the criterion being looked at.
  useEffect(() => {
    setDetail(null);
    if (!slug || !pickedTask) return;
    let live = true;
    api.evidenceDetail(slug, pickedTask).then((d) => { if (live) setDetail(d); }, () => { if (live) setDetail("failed"); });
    return () => { live = false; };
  }, [slug, pickedTask]);

  if (!slug) {
    return (
      <div className="v-home">
        <Text as="h1" variant="headline-lg">{S.run.title}</Text>
        <Text variant="body-sm" muted>{S.run.pick}</Text>
      </div>
    );
  }
  return (
    <div className="v-home">
      <Text as="h1" variant="headline-lg">{S.run.title}</Text>
      <Section title={S.run.evidence}>
        {err && <div role="alert"><Text variant="body-sm">{S.run.failed} {err}</Text></div>}
        {rows && rows.length === 0 && <Text variant="body-sm" muted>{S.run.none}</Text>}
        {rows && rows.length > 0 && (
          <>
            {/* Green only when every criterion is a pass (POS-1.5). */}
            {passing === sorted.length
              ? <Badge status="success">{S.run.allPass}</Badge>
              : <Badge status="danger">{S.run.summary(passing, sorted.length)}</Badge>}
            <VirtualList
              role="list"
              aria-label={S.run.criteria}
              count={sorted.length}
              rowHeight={ROW_H}
              height={LIST_H}
              renderRow={(i) => {
                const r = sorted[i];
                return (
                  <div role="listitem" className="v-run__row">
                    <Button size="sm" variant="ghost" onClick={() => setSel(keyOf(r))}>{r.criterion}</Button>
                    <span>{r.task}</span>
                    <Badge status={BADGE[r.status]}>{label(r.status)}</Badge>
                  </div>
                );
              }}
            />
          </>
        )}
        {picked && (
          <dl className="v-crew__facts" role="group" aria-label={S.run.detail}>
            <dt>{S.run.criterion}</dt><dd>{picked.criterion}</dd>
            <dt>{S.run.taskCol}</dt><dd>{picked.task} {picked.title ?? ""}</dd>
            {picked.owner && (<><dt>{S.run.writtenBy}</dt><dd>{picked.owner}</dd></>)}
            <dt>{S.run.statusCol}</dt><dd>{label(picked.status)}</dd>
            {picked.path || picked.head ? (
              <>
                <dt>{S.run.commit}</dt><dd>{picked.head ? picked.head.slice(0, 7) : ""}</dd>
                <dt>{S.run.evidenceFile}</dt><dd>{picked.path ?? ""}</dd>
                <dt>{S.run.commandsRun}</dt>
                <dd>
                  {detail === null && <Text variant="body-sm" muted>…</Text>}
                  {detail === "failed" && <Text variant="body-sm">{S.run.detailFailed}</Text>}
                  {detail && detail !== "failed" && detail.truncated && <Text variant="body-sm" muted>{S.run.detailTruncated}</Text>}
                  {detail && detail !== "failed" && !detail.truncated && detail.commands.length === 0 && <Text variant="body-sm" muted>{S.run.noCommands}</Text>}
                  {detail && detail !== "failed" && detail.commands.map((c, k) => (
                    <div key={k} className="v-run__cmd">
                      {/* Text nodes only: output is untrusted and never rendered as markup. */}
                      <code>{c.cmd ?? ""}</code> <Badge status={c.passed ? "success" : "danger"}>{S.run.exit(c.exit_code)}</Badge>
                      {c.cmd && <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(c.cmd ?? "")}>{S.crew.copy(S.run.copyWhat)}</Button>}
                      {c.output_tail && <pre aria-label={S.run.output}>{c.output_tail}</pre>}
                    </div>
                  ))}
                </dd>
              </>
            ) : (<><dt>{S.run.evidenceCol}</dt><dd>{S.run.noEvidence}</dd></>)}
            {picked.report && (<><dt>{S.run.report}</dt><dd>{picked.report}</dd></>)}
            {detail && detail !== "failed" && detail.notes && detail.notes !== picked.report && (<><dt>{S.run.notes}</dt><dd>{detail.notes}</dd></>)}
          </dl>
        )}
      </Section>
    </div>
  );
}
