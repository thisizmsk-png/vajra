// The main area's header: the project's views (Director-General, Board, Review) and one tab per worker you opened.
// They are links with aria-current, not an ARIA tablist: each one changes the route, and Back works.
import { Icon, ariaShortcut } from "../../components/ui/base";
import { S } from "../../strings";
import { toHash } from "../router";
import type { Route } from "../router";

export type WorkerTab = { slug: string; id: string; worker: string; name: string };

export function ViewTabs({ route, project, review, workers, onClose, inspector, onInspector }: {
  route: Route;
  project: string | null;
  /** Params for Review: the campaign the project is on, so the tab opens on something. */
  review: Record<string, string>;
  workers: WorkerTab[];
  onClose: (t: WorkerTab) => void;
  inspector: boolean | null;
  onInspector: () => void;
}) {
  const here = (s: string) => (route.screen === s ? "page" : undefined);
  const views = [
    { id: "orch", label: S.views.orch, href: toHash("orch", {}, project) },
    { id: "board", label: S.views.board, href: toHash("board", {}, project) },
    { id: "review", label: S.views.review, href: toHash("review", review, project) },
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
        <button type="button" className="v-icon-btn" aria-pressed={inspector} aria-label={inspector ? S.frame.hideInspector : S.frame.showInspector}
          title={`${inspector ? S.frame.hideInspector : S.frame.showInspector} (⌘I)`} aria-keyshortcuts={ariaShortcut(["⌘", "I"])} onClick={onInspector}>
          <Icon name="panelRight" />
        </button>
      )}
    </div>
  );
}
