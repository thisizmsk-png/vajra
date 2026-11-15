// The main area's header: the project's views (Director-General, Board, Review) and one tab per worker you opened.
// They are links with aria-current, not an ARIA tablist: each one changes the route, and Back works.
import { Icon, ariaShortcut } from "../../components/ui/base";
import { S } from "../../strings";
import { toHash } from "../router";
import type { Route } from "../router";
import type { InboxItem } from "../ipc";

/** The Inbox tab's count: campaign items only, since a run proposal is shown on the Director-General surface, not in the
 * Inbox. `campaignCount` is the core's own campaign-only total (`campaign_count`): when it is there, the badge is exactly
 * that. An older core sends only `count`, the total over every kind; then, when it sent fewer items than it counts, the
 * campaign total is not known and more may be waiting past the cut, so the badge shows a "+": the count it has ("2+"),
 * or a bare "+" when every sent item was a proposal (PR 90 review), never "0+" and never nothing. Empty means no badge. */
export function inboxBadge(items: readonly InboxItem[], count: number, campaignCount: number | null = null): string {
  if (campaignCount !== null) return campaignCount > 0 ? String(campaignCount) : "";
  const campaign = items.filter((i) => i.kind !== "run_proposal").length;
  if (count > items.length) return campaign > 0 ? `${campaign}+` : "+";
  return campaign > 0 ? String(campaign) : "";
}

export type WorkerTab = { slug: string; id: string; worker: string; name: string };

export function ViewTabs({ route, project, review, workers, waiting = "", onClose, inspector, onInspector }: {
  route: Route;
  project: string | null;
  /** Params for Review: the campaign the project is on, so the tab opens on something. */
  review: Record<string, string>;
  workers: WorkerTab[];
  /** The Inbox tab's count from `inboxBadge` ("2", "20+"); empty shows no count. */
  waiting?: string;
  onClose: (t: WorkerTab) => void;
  inspector: boolean | null;
  onInspector: () => void;
}) {
  const here = (s: string) => (route.screen === s || (s === "orch" && route.screen === "launch") ? "page" : undefined);
  const views = [
    { id: "orch", label: S.views.orch, href: toHash("orch", {}, project) },
    { id: "board", label: S.views.board, href: toHash("board", {}, project) },
    { id: "review", label: S.views.review, href: toHash("review", review, project) },
    { id: "inbox", label: waiting ? `${S.views.inbox} ${waiting}` : S.views.inbox, href: toHash("inbox", {}, project) },
  ];
  return (
    <div className="v-mh" data-tauri-drag-region>
      <nav className="v-vtabs" aria-label={S.frame.views}>
        {views.map((v) => (
          <a key={v.id} href={v.href} className="v-vtab" aria-current={here(v.id)}>{v.label}</a>
        ))}
        {workers.map((w) => {
          const on = route.screen === "worker" && route.params.worker === w.worker && route.params.id === w.id;
          return (
            <span key={`${w.id}/${w.worker}`} className="v-vtab v-vtab--worker" data-current={on || undefined}>
              <a href={toHash("worker", { slug: w.slug, id: w.id, worker: w.worker }, project)} aria-current={on ? "page" : undefined}>{w.name}</a>
              <button type="button" className="v-vtab__x" aria-label={S.frame.closeTab(w.name)} onClick={() => onClose(w)}><Icon name="close" size={12} /></button>
            </span>
          );
        })}
      </nav>
      {inspector !== null && (
        <button type="button" className="v-icon-btn" aria-pressed={inspector} aria-label={S.frame.inspector}
          title={`${inspector ? S.frame.hideInspector : S.frame.showInspector} (⌘I)`} aria-keyshortcuts={ariaShortcut(["⌘", "I"])} onClick={onInspector}>
          <Icon name="panelRight" />
        </button>
      )}
    </div>
  );
}
