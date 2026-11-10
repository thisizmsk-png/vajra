// Which component renders each view. A slice replaces its entry; nothing else in the shell changes.
// The Director-General view is its conversation (FG-02); the run launcher is its own view, "Start a run yourself".
import type { ComponentType } from "react";
import type { Screen } from "../router";
import { Home } from "./home/Home";
import { DirectorGeneral } from "./orch/DirectorGeneral";
import { Board } from "./board/Board";
import { Crew } from "./crew/Crew";
import { Run } from "./run/Run";
import { History } from "./history/History";
import { Inbox } from "./inbox/Inbox";
import { Settings } from "./settings/Settings";
import { WorkerView } from "./worker/WorkerView";

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
