// Which component renders each view. A slice replaces its entry; nothing else in the shell changes.
// The Director-General view shows the run launcher until its conversation (FG-04) lands in this slot.
import type { ComponentType } from "react";
import type { Screen } from "../router";
import { Home } from "./home/Home";
import { Board } from "./board/Board";
import { Crew } from "./crew/Crew";
import { Run } from "./run/Run";
import { History } from "./history/History";
import { Inbox } from "./inbox/Inbox";
import { Settings } from "./settings/Settings";
import { WorkerView } from "./worker/WorkerView";

export const REGISTRY: Record<Screen, ComponentType> = {
  orch: Home,
  board: Board,
  review: Run,
  worker: WorkerView,
  crew: Crew,
  history: History,
  inbox: Inbox,
  settings: Settings,
};
