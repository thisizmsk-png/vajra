// Which component renders each screen. A screen PR replaces its entry; nothing else in the shell changes.
import type { ComponentType } from "react";
import { Text } from "../../components/ui/base";
import { S } from "../../strings";
import type { Screen } from "../router";
import { Home } from "./home/Home";
import { Crew } from "./crew/Crew";
import { Run } from "./run/Run";
import { History } from "./history/History";
import { Settings } from "./settings/Settings";

const notBuilt = (screen: Screen): ComponentType => function NotBuilt() {
  return (
    <section aria-label={S.nav[screen]}>
      <Text as="h1" variant="headline-lg">{S.nav[screen]}</Text>
      <Text variant="body-sm">{S.shell.notBuilt}</Text>
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
