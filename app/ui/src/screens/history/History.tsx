import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Input, Section, Text } from "../../../components/ui/base";
import { Table } from "../../../components/ui/desktop";
import type { Column } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, onChanged } from "../../ipc";
import type { Campaign } from "../../ipc";
import { useRoute } from "../../router";

type Hit = { id: string; kind: string; title: string };
type Result = { state: "idle" } | { state: "busy" } | { state: "ok"; hits: Hit[] } | { state: "unavailable"; code: number | null } | { state: "error"; text: string };

const text = (a?: string, b?: string) => (a ?? "").localeCompare(b ?? "");
const COLUMNS = (open: (c: Campaign) => void): Column<Campaign>[] => [
  { key: "slug", header: S.history.colSlug, sort: (a, b) => text(a.slug, b.slug), cell: (c) => <Button size="sm" variant="ghost" onClick={() => open(c)}>{c.slug}</Button> },
  { key: "goal", header: S.history.colGoal, sort: (a, b) => text(a.goal, b.goal), cell: (c) => c.error ? <Badge status="danger">{S.history.unreadable}: {c.error}</Badge> : c.goal ?? "" },
  { key: "status", header: S.history.colStatus, sort: (a, b) => text(a.status, b.status), cell: (c) => c.status ?? "" },
  { key: "passing", header: S.history.colPassing, numeric: true, sort: (a, b) => (a.passing ?? 0) - (b.passing ?? 0), cell: (c) => (c.total != null ? `${c.passing ?? 0}/${c.total}` : "") },
  { key: "updated", header: S.history.colUpdated, sort: (a, b) => text(a.updated, b.updated), cell: (c) => c.updated ?? "" },
];

export function History() {
  const [, go] = useRoute();
  const [camps, setCamps] = useState<Campaign[] | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [q, setQ] = useState("");
  const [repo, setRepo] = useState("");
  const [shown, setShown] = useState(false);
  const [res, setRes] = useState<Result>({ state: "idle" });

  const load = useCallback(() => {
    api.listCampaigns().then((r) => { setCamps(r.campaigns); setLoadErr(""); }, (e) => setLoadErr(errorText(e)));
  }, []);
  useEffect(load, [load]);
  useEffect(() => onChanged(load), [load]);

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
          <Table caption={S.history.campaigns} columns={COLUMNS((c) => go("run", { slug: c.slug }))} rows={camps} rowKey={(c) => c.slug} />
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
