// Which component renders each view. A slice replaces its entry; nothing else in the shell changes.
// The Director-General view is its conversation (FG-02); the run launcher is its own view, "Start a run yourself".
import { lazy, Suspense } from "react";
import type { ComponentType } from "react";
import { S } from "../../strings";
import type { Screen } from "../router";
import { Home } from "./home/Home";
import { DirectorGeneral } from "./orch/DirectorGeneral";
import { Board } from "./board/Board";
import { Crew } from "./crew/Crew";
import { Run } from "./run/Run";
import { History } from "./history/History";
import { Inbox } from "./inbox/Inbox";
import { LoadingRows } from "./states";
import { WorkerView } from "./worker/WorkerView";

// Settings (with the accounts and project-agents panels) is opened rarely, so it is its own chunk: the idle shell never
// loads or parses it (the idle memory gate counts every kB). The fallback is the design system's own quiet skeleton rows,
// which only appear after their delay, so a fast load never flashes.
const SettingsChunk = lazy(() => import("./settings/Settings").then((m) => ({ default: m.Settings })));
function Settings() {
  return (
    <Suspense fallback={<div className="v-screen"><div className="v-screen__body"><LoadingRows rows={4} label={S.states.loading} /></div></div>}>
      <SettingsChunk />
    </Suspense>
  );
}

export const REGISTRY: Record<Screen, ComponentType> = {
  orch: DirectorGeneral,
  launch: Home,
  board: Board,
  review: Run,
  worker: WorkerView,
  crew: Crew,
  history: History,
  inbox: Inbox,
  settings: Settings,
};
