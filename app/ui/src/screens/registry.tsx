// Which component renders each screen. A screen PR replaces its entry; nothing else in the shell changes.
import type { ComponentType } from "react";
import { Button, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { useRoute } from "../router";
import type { Screen } from "../router";
import { Home } from "./home/Home";
import { Crew } from "./crew/Crew";
import { Run } from "./run/Run";
import { History } from "./history/History";
import { Settings } from "./settings/Settings";
import { Empty } from "./states";

/** A screen that is not built yet says so plainly. The Run view, opened right after a start, points at the crew. */
const notBuilt = (screen: Screen): ComponentType => function NotBuilt() {
  const [route, go] = useRoute();
  const run = screen === "run" && route.params.slug && route.params.id ? { slug: route.params.slug, id: route.params.id } : null;
  return (
    <section aria-label={S.nav[screen]} className="v-screen">
      <header className="v-screen__head"><Text as="h1" variant="label">{S.nav[screen]}</Text></header>
      <div className="v-screen__body v-crew__state">
        <Empty title={S.shell.notBuilt}
          action={run ? <Button variant="primary" onClick={() => go("crew", run)}>{S.states.openCrew}</Button>
            : <Button onClick={() => go("home")}>{S.states.goHome}</Button>}>
          {run ? S.states.runStarted : undefined}
        </Empty>
      </div>
    </section>
  );
};

export const REGISTRY: Record<Screen, ComponentType> = {
  home: Home,
  run: Run,
  crew: Crew,
  chat: notBuilt("chat"),
  inbox: notBuilt("inbox"),
  history: History,
  settings: Settings,
};
