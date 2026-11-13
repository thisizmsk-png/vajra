import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, Icon, Input, Text } from "../../../components/ui/base";
import { ActionBar, Table } from "../../../components/ui/desktop";
import type { Column } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import { Empty, LoadingRows, Problem, problemDetail } from "../states";
import { useCampaignIntegrity } from "../campaignIntegrity";
import type { Campaign, Report } from "../../ipc";
import { useRoute } from "../../router";
import { ago } from "../when";

type Hit = { id: string; kind: string; title: string };
type Result = { state: "idle" } | { state: "busy" } | { state: "ok"; hits: Hit[] } | { state: "unavailable"; code: number | null } | { state: "error"; text: string };

/** Show control characters (NUL, ESC, ...) and every invisible character (bidi marks, overrides and isolates, zero-width
 * and line separators, soft hyphen, fillers, variation selectors, and the tag block that spells ASCII invisibly) as
 * visible escapes, so a report cannot reorder or hide what it says; newline and tab stay. The set matches the core's
 * `control::lead::invisible`. Code points above U+FFFF are written `\u{...}`. */
export const escapeControls = (t: string) =>
  t.replace(
    /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u00ad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180f\u200b-\u200f\u2028-\u202e\u2060-\u206f\u3164\ufe00-\ufe0f\ufeff\uffa0\ufff0-\ufffb\u{1bca0}-\u{1bca3}\u{1d173}-\u{1d17a}\u{e0000}-\u{e0fff}]/gu,
    (c) => {
      const n = c.codePointAt(0)!;
      return n > 0xffff ? `\\u{${n.toString(16)}}` : `\\u${n.toString(16).padStart(4, "0")}`;
    },
  );

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
const TONE = { active: "info", done: "success", stopped: "warning", failed: "danger", abandoned: undefined } as Record<string, "success" | "warning" | "danger" | "info" | undefined>;
const COLUMNS = (open: (c: Campaign) => void, report: (c: Campaign) => void, verify: (c: Campaign) => void): Column<Campaign>[] => [
  // The goal leads; the slug under it opens the run and names the button (the slug is what the terminal shows).
  { key: "slug", header: S.history.colSlug, sort: (a, b) => text(a.slug, b.slug), cell: (c) => (
    <div className="v-history__camp">
      {c.error ? <Badge status="danger">{S.history.unreadable}: {c.error}</Badge> : <span className="v-history__goal" title={c.goal}>{c.goal ?? ""}</span>}
      <button type="button" className="v-history__slug" title={c.slug} onClick={() => open(c)}>{c.slug}</button>
    </div>
  ) },
  { key: "status", header: S.history.colStatus, sort: (a, b) => text(a.status, b.status), cell: (c) => (c.status ? <Badge status={TONE[c.status]}>{S.history.state[c.status] ?? c.status}</Badge> : null) },
  { key: "passing", header: S.history.colPassing, sort: (a, b) => (a.passing ?? 0) / (a.total || 1) - (b.passing ?? 0) / (b.total || 1), cell: (c) => (c.total != null ? (
    <span className="v-history__pass">
      <span className="v-history__meter" aria-hidden="true"><span style={{ width: `${c.total ? Math.round(((c.passing ?? 0) / c.total) * 100) : 0}%` }} /></span>
      <span className="v-num" aria-label={S.history.of(c.passing ?? 0, c.total)}>{`${c.passing ?? 0}/${c.total}`}</span>
    </span>
  ) : "") },
  { key: "updated", header: S.history.colUpdated, sort: (a, b) => text(a.updated, b.updated), cell: (c) => <span className="v-history__muted" title={c.updated}>{ago(c.updated)}</span> },
  { key: "integrity", header: S.history.colIntegrity, sort: (a, b) => text(a.integrity, b.integrity), cell: (c) => (
    c.integrity === "checking" ? <Badge>{S.history.integrityChecking}</Badge>
      : c.integrity === "failed" ? <Badge status="danger">{S.history.integrityFailed}</Badge>
      : c.integrity === "ok" ? <Badge status="success">{S.history.integrityOk}</Badge>
      : <Badge>{S.history.integrityUnknown}</Badge>
  ) },
  { key: "actions", header: S.history.colActions, cell: (c) => c.error ? null : (
    // The visible word starts the accessible name, which adds the campaign (WCAG 2.5.3).
    <span className="v-history__acts">
      <Button size="sm" variant="secondary" aria-label={S.history.report(c.slug)} onClick={() => report(c)}>{S.history.reportShort}</Button>
      <Button size="sm" variant="ghost" aria-label={S.history.verify(c.slug)} onClick={() => verify(c)}>{S.history.verifyShort}</Button>
    </span>
  ) },
];

export function History() {
  const [, go] = useRoute();
  const [loaded, setCamps] = useState<Campaign[] | null>(null);
  const camps = useCampaignIntegrity(loaded); // the list as loaded, with each "checking" badge filled in as its call returns
  const [loadErr, setLoadErr] = useState("");
  const [q, setQ] = useState("");
  const [repo, setRepo] = useState("");
  const [shown, setShown] = useState(false);
  const [view, setView] = useState<View | null>(null);
  const [res, setRes] = useState<Result>({ state: "idle" });

  const load = useCallback(() => {
    api.listCampaigns().then((r) => { setCamps(r.campaigns); setLoadErr(""); }, (e) => setLoadErr(problemDetail(e)));
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
    // The core settles the campaign's shared integrity verdict from this very check (and emits no changed event), so
    // reload the table even when this reply is superseded: the row badge then shows the verdict this check settled.
    load();
    if (mine === req.current) setView(next);
  };
  // Closing the pane ends the current request's claim on it: its reply still reloads the table (see `run`) but cannot reopen the pane.
  const closeView = () => { req.current++; setView(null); };
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
    <div className="v-screen v-history">
      <header className="v-screen__head" data-tauri-drag-region>
        <Text as="h1" variant="label">{S.history.title}</Text>
        {camps && <span className="v-screen__meta">{S.history.count(camps.length)}</span>}
      </header>
      <div className={view ? "v-history__body v-history__body--open" : "v-history__body"}>
        <div className="v-screen__body v-history__main">
          <section aria-labelledby="v-history-camps" className="v-panel">
            <div className="v-panel__bar">
              <h2 id="v-history-camps" className="v-panel__title">{S.history.campaigns}</h2>
            </div>
            {(loadErr || !camps || camps.length === 0) && (
              <div className="v-panel__body">
                {loadErr && <Problem title={S.history.failed} detail={loadErr} retry={load} />}
                {!camps && !loadErr && <LoadingRows label={S.states.loading} />}
                {camps && camps.length === 0 && (
                  <Empty title={S.history.empty} action={<Button onClick={() => go("home")}>{S.states.goHome}</Button>}>{S.history.emptyHint}</Empty>
                )}
              </div>
            )}
            {camps && camps.length > 0 && (
              <Table caption={S.history.campaigns} columns={COLUMNS((c) => go("run", { slug: c.slug }), (c) => void showReport(c), (c) => void verifyNow(c))} rows={camps} rowKey={(c) => c.slug} />
            )}
          </section>
          <section aria-labelledby="v-history-search" className="v-panel v-history__search">
            <div className="v-panel__bar"><h2 id="v-history-search" className="v-panel__title">{S.history.search}</h2></div>
            <div className="v-panel__body">
            <form className="v-history__form" onSubmit={(e) => { e.preventDefault(); void search(); }} noValidate>
              <div className="v-history__q"><Input label={S.history.query} hint={S.history.queryHint} value={q} onChange={setQ} error={shown && !q.trim() ? S.history.errQuery : undefined} /></div>
              <div className="v-history__repo"><Input label={S.history.repo} value={repo} onChange={setRepo} error={shown && !repo.trim() ? S.history.errRepo : undefined} /></div>
              <Button variant="primary" className="v-history__go" onClick={() => void search()} busy={res.state === "busy"}>{res.state === "busy" ? S.history.searching : S.history.go}</Button>
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
            </div>
          </section>
        </div>
        {view && (
          <aside className="v-history__pane" aria-label={S.history.panel}>
            <div className="v-history__panehead">
              <Text as="h2" variant="label">{S.history.panel}</Text>
              <span className="v-history__paneslug">{view.slug}</span>
              <Button size="sm" variant="ghost" aria-label={S.history.closeReport} onClick={closeView}><Icon name="close" /></Button>
            </div>
            <div className="v-history__panebody" aria-live="polite">
              {view.state === "busy" && <Text variant="body-sm" muted>{S.history.reportLoading}</Text>}
              {view.state === "error" && <div role="alert"><Text variant="body-sm">{view.text}</Text></div>}
              {view.state === "verified" && (view.ok
                ? <Badge status="success">{S.history.verifyOk}</Badge>
                : <div role="alert"><Text variant="body-sm">{view.restore ? S.history.verifyFailed : S.history.verifyFailedNoCheckpoint}</Text>{view.restore && <pre className="v-home__report">{view.restore}</pre>}</div>)}
              {view.state === "done" && reportBadge(view.report.integrity)}
              {view.state === "done" && (view.report.integrity === "ok" && view.report.text != null
                ? <section aria-label={S.history.reportTitle(view.slug)}><pre className="v-home__report">{escapeControls(view.report.text)}</pre></section>
                : <div role="status"><Text variant="body-sm">{REPORT_MESSAGE[view.report.integrity] ?? S.history.reportUnknown}</Text></div>)}
            </div>
          </aside>
        )}
      </div>
      {view && <ActionBar label={S.history.barLabel} context={<span>{S.history.reportTitle(view.slug)}</span>} />}
    </div>
  );
}
