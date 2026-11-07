import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, Input, Section, Text } from "../../../components/ui/base";
import { Table } from "../../../components/ui/desktop";
import type { Column } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { Campaign, Report } from "../../ipc";
import { useRoute } from "../../router";

type Hit = { id: string; kind: string; title: string };
type Result = { state: "idle" } | { state: "busy" } | { state: "ok"; hits: Hit[] } | { state: "unavailable"; code: number | null } | { state: "error"; text: string };

/** Show control characters (NUL, ESC, ...) and the invisible direction and line-separator marks (bidi overrides and
 * isolates, LRM/RLM, U+2028/9) as visible \uXXXX escapes, so a report cannot reorder or hide what it says; newline
 * and tab stay. */
export const escapeControls = (t: string) =>
  t.replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u200e\u200f\u2028\u2029\u202a-\u202e\u2066-\u2069]/g,
    (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`);

/** The badge for one report read. It comes from that read's own result (the held bytes it checked), never from the
 * listing: no_report and not_text passed the signature check, so they read as verified too. */
const reportBadge = (i: Report["integrity"]) =>
  i === "integrity_failed" ? <Badge status="danger">{S.history.integrityFailed}</Badge>
    : i === "unknown" ? <Badge>{S.history.integrityUnknown}</Badge>
    : <Badge status="success">{S.history.integrityOk}</Badge>;

type View = { slug: string; state: "busy" } | { slug: string; state: "done"; report: Report } | { slug: string; state: "error"; text: string }
  | { slug: string; state: "verified"; ok: boolean; restore: string | null };
const REPORT_MESSAGE: Record<string, string> = {
  no_report: S.history.noReport, integrity_failed: S.history.reportFailed, not_text: S.history.notText, unknown: S.history.reportUnknown,
};

const text = (a?: string, b?: string) => (a ?? "").localeCompare(b ?? "");
const COLUMNS = (open: (c: Campaign) => void, report: (c: Campaign) => void, verify: (c: Campaign) => void): Column<Campaign>[] => [
  { key: "slug", header: S.history.colSlug, sort: (a, b) => text(a.slug, b.slug), cell: (c) => <Button size="sm" variant="ghost" onClick={() => open(c)}>{c.slug}</Button> },
  { key: "goal", header: S.history.colGoal, sort: (a, b) => text(a.goal, b.goal), cell: (c) => c.error ? <Badge status="danger">{S.history.unreadable}: {c.error}</Badge> : c.goal ?? "" },
  { key: "status", header: S.history.colStatus, sort: (a, b) => text(a.status, b.status), cell: (c) => c.status ?? "" },
  { key: "passing", header: S.history.colPassing, numeric: true, sort: (a, b) => (a.passing ?? 0) - (b.passing ?? 0), cell: (c) => (c.total != null ? `${c.passing ?? 0}/${c.total}` : "") },
  { key: "updated", header: S.history.colUpdated, sort: (a, b) => text(a.updated, b.updated), cell: (c) => c.updated ?? "" },
  { key: "integrity", header: S.history.colIntegrity, sort: (a, b) => text(a.integrity, b.integrity), cell: (c) => (
    c.integrity === "failed" ? <Badge status="danger">{S.history.integrityFailed}</Badge>
      : c.integrity === "ok" ? <Badge status="success">{S.history.integrityOk}</Badge>
      : <Badge>{S.history.integrityUnknown}</Badge>
  ) },
  { key: "actions", header: S.history.colActions, cell: (c) => c.error ? null : (
    <>
      <Button size="sm" variant="ghost" onClick={() => report(c)}>{S.history.report(c.slug)}</Button>
      <Button size="sm" variant="ghost" onClick={() => verify(c)}>{S.history.verify(c.slug)}</Button>
    </>
  ) },
];

export function History() {
  const [, go] = useRoute();
  const [camps, setCamps] = useState<Campaign[] | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [q, setQ] = useState("");
  const [repo, setRepo] = useState("");
  const [shown, setShown] = useState(false);
  const [view, setView] = useState<View | null>(null);
  const [res, setRes] = useState<Result>({ state: "idle" });

  const load = useCallback(() => {
    api.listCampaigns().then((r) => { setCamps(r.campaigns); setLoadErr(""); }, (e) => setLoadErr(errorText(e)));
  }, []);
  useEffect(load, [load]);
  useEffect(() => onChanged(load), [load]);

  // Only the newest report or verify request may set the view: a slow reply for an earlier click is dropped.
  const req = useRef(0);
  const run = async (c: Campaign, act: () => Promise<View>) => {
    const mine = ++req.current;
    setView({ slug: c.slug, state: "busy" });
    let next: View;
    try {
      next = await act();
    } catch (e) {
      next = { slug: c.slug, state: "error", text: errorText(e) };
    }
    if (mine !== req.current) return;
    setView(next);
    // The core settles the campaign's shared integrity verdict from this very check, so reload the table: the row
    // badge then shows the same verdict as the report or verify result, not an older one.
    load();
  };
  const showReport = (c: Campaign) => run(c, async () => ({ slug: c.slug, state: "done", report: await api.readReport(c.slug) }));
  const verifyNow = (c: Campaign) => run(c, async () => {
    const v = await api.verifyCampaign(c.slug);
    return { slug: c.slug, state: "verified", ok: v.ok, restore: v.restore ?? null };
  });

  const search = async () => {
    setShown(true);
    if (!q.trim() || !repo.trim()) return;
    setRes({ state: "busy" });
    try {
      const r = await api.kbSearch(q, 20, repo.trim());
      setRes(r.code === 0 ? { state: "ok", hits: r.hits as Hit[] } : { state: "unavailable", code: r.code });
    } catch (e) {
      setRes({ state: "error", text: errorText(e) });
    }
  };

  return (
    <div className="v-home">
      <Text as="h1" variant="headline-lg">{S.history.title}</Text>
      <Section title={S.history.campaigns}>
        {loadErr && <div role="alert"><Text variant="body-sm">{S.history.failed} {loadErr}</Text></div>}
        {camps && camps.length === 0 && <Text variant="body-sm" muted>{S.history.empty}</Text>}
        {camps && camps.length > 0 && (
          <Table caption={S.history.campaigns} columns={COLUMNS((c) => go("run", { slug: c.slug }), (c) => void showReport(c), (c) => void verifyNow(c))} rows={camps} rowKey={(c) => c.slug} />
        )}
        {view && (
          <div aria-live="polite">
            {view.state === "busy" && <Text variant="body-sm" muted>{S.history.reportLoading}</Text>}
            {view.state === "error" && <div role="alert"><Text variant="body-sm">{view.text}</Text></div>}
            {view.state === "verified" && (view.ok
              ? <Text variant="body-sm">{S.history.verifyOk}</Text>
              : <div role="alert"><Text variant="body-sm">{view.restore ? S.history.verifyFailed : S.history.verifyFailedNoCheckpoint}</Text>{view.restore && <pre className="v-home__report">{view.restore}</pre>}</div>)}
            {view.state === "done" && reportBadge(view.report.integrity)}
            {view.state === "done" && (view.report.integrity === "ok" && view.report.text != null
              ? <section aria-label={S.history.reportTitle(view.slug)}><pre className="v-home__report">{escapeControls(view.report.text)}</pre></section>
              : <div role="status"><Text variant="body-sm">{REPORT_MESSAGE[view.report.integrity] ?? S.history.reportUnknown}</Text></div>)}
          </div>
        )}
      </Section>
      <Section title={S.history.search}>
        <form className="v-home__form" onSubmit={(e) => { e.preventDefault(); void search(); }} noValidate>
          <Input label={S.history.query} hint={S.history.queryHint} value={q} onChange={setQ} error={shown && !q.trim() ? S.history.errQuery : undefined} />
          <Input label={S.history.repo} value={repo} onChange={setRepo} error={shown && !repo.trim() ? S.history.errRepo : undefined} />
          <Button variant="primary" onClick={() => void search()} busy={res.state === "busy"}>{res.state === "busy" ? S.history.searching : S.history.go}</Button>
        </form>
        {res.state === "ok" && res.hits.length === 0 && <Text variant="body-sm" muted>{S.history.none}</Text>}
        {res.state === "ok" && res.hits.length > 0 && (
          <ul className="v-home__list" aria-label={S.history.search}>
            {res.hits.map((h) => (
              <li key={h.id}><span>{h.title}</span><Badge>{h.kind}</Badge></li>
            ))}
          </ul>
        )}
        {res.state === "unavailable" && <div role="alert"><Text variant="body-sm">{S.history.unavailable(res.code)}</Text></div>}
        {res.state === "error" && <div role="alert"><Text variant="body-sm">{res.text}</Text></div>}
      </Section>
    </div>
  );
}
