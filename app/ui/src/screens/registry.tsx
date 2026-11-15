// Which component renders each view. A slice replaces its entry; nothing else in the shell changes.
// The Director-General view is its conversation (FG-02); the run launcher is its own view, "Start a run yourself".
import { lazy, Suspense } from "react";
import type { ComponentType } from "react";
import { S } from "../../strings";
import type { Screen } from "../router";
import { Home } from "./home/Home";
import { DirectorGeneral } from "./orch/DirectorGeneral";
import { LoadingRows } from "./states";

// Only the landing view (the Director-General conversation) is in the start bundle. Every other view is its own chunk,
// loaded the first time it is opened: the idle shell never loads or parses it (the idle memory gate counts every kB).
// The fallback is the design system's own quiet skeleton rows, which only appear after their delay, so a fast load never
// flashes.
function onOpen(load: () => Promise<{ default: ComponentType }>): ComponentType {
  const Chunk = lazy(load);
  return function Screen() {
    return (
      <Suspense fallback={<div className="v-screen"><div className="v-screen__body"><LoadingRows rows={4} label={S.states.loading} /></div></div>}>
        <Chunk />
      </Suspense>
    );
  };
}
const Board = onOpen(() => import("./board/Board").then((m) => ({ default: m.Board })));
const Crew = onOpen(() => import("./crew/Crew").then((m) => ({ default: m.Crew })));
const Run = onOpen(() => import("./run/Run").then((m) => ({ default: m.Run })));
const History = onOpen(() => import("./history/History").then((m) => ({ default: m.History })));
const Inbox = onOpen(() => import("./inbox/Inbox").then((m) => ({ default: m.Inbox })));
const WorkerView = onOpen(() => import("./worker/WorkerView").then((m) => ({ default: m.WorkerView })));
const Settings = onOpen(() => import("./settings/Settings").then((m) => ({ default: m.Settings })));

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
