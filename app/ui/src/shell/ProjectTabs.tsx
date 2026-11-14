// The title bar's project tabs: one per project, with worker pips and the urgent state (needs you, stopped, failed).
import { Badge, Icon, ariaShortcut } from "../../components/ui/base";
import { S } from "../../strings";
import type { Project, ProjectSummary } from "../projects/contract";
import { toHash } from "../router";
import type { Screen } from "../router";

/** Views that make sense in any project; a worker or a campaign-specific view lands on the Director-General. */
const KEEP: ReadonlySet<Screen> = new Set(["orch", "launch", "board", "history"]);
const MAX_PIPS = 6;

export function stateText(s: ProjectSummary | undefined): string {
  if (!s?.state) return "";
  return s.state === "needs_you" ? S.frame.needsYou(s.needsYou) : s.state === "stopped" ? S.frame.stopped : S.frame.failed;
}

export function ProjectTabs({ projects, summaries, current, view, onAdd }: {
  projects: Project[]; summaries: Map<string, ProjectSummary>; current: string | null; view: Screen; onAdd: () => void;
}) {
  const target: Screen = KEEP.has(view) ? view : "orch";
  return (
    <>
      <nav className="v-ptabs" aria-label={S.frame.projects}>
        {projects.map((p, i) => {
          const s = summaries.get(p.id);
          const state = stateText(s);
          return (
            <a key={p.id} href={toHash(target, {}, p.id)} className={p.bucket ? "v-ptab v-ptab--bucket" : "v-ptab"}
              aria-current={p.id === current && view !== "settings" ? "page" : undefined}
              aria-label={S.frame.tabName(p.name, state)} title={p.root ?? S.projects.unsortedHint}
              aria-keyshortcuts={i < 9 ? ariaShortcut(["⌘", String(i + 1)]) : undefined}>
              <span className="v-pmark" aria-hidden="true">{p.bucket ? "·" : p.name.slice(0, 1).toUpperCase()}</span>
              <span className="v-ptab__name">{p.name}</span>
              {s && s.pips.length > 0 && (
                <span className="v-pips" aria-hidden="true">
                  {s.pips.slice(0, MAX_PIPS).map((k, j) => <i key={j} className={`v-status--${k}`} />)}
                </span>
              )}
              {s?.state === "needs_you" && <Badge status="warning" className="v-ptab__pill">{state}</Badge>}
              {(s?.state === "stopped" || s?.state === "failed") && <span className={`v-ptab__state v-status--${s.state === "stopped" ? "blocked" : "failed"}`}>{state}</span>}
            </a>
          );
        })}
      </nav>
      <button type="button" className="v-icon-btn" aria-label={S.frame.addProject} title={S.frame.addProjectHint} onClick={onAdd}>
        <Icon name="plus" />
      </button>
    </>
  );
}
