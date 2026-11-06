// Which component renders each screen. A screen PR replaces its entry; nothing else in the shell changes.
import type { ComponentType } from "react";
import { Text } from "../../components/ui/base";
import { S } from "../../strings";
import type { Screen } from "../router";

const notBuilt = (screen: Screen): ComponentType => function NotBuilt() {
  return (
    <section aria-labelledby={`h-${screen}`}>
      <Text as="h1" variant="headline-lg" id={`h-${screen}`}>{S.nav[screen]}</Text>
      <Text variant="body-sm">{S.shell.notBuilt}</Text>
    </section>
  );
};

export const REGISTRY: Record<Screen, ComponentType> = {
  home: notBuilt("home"),
  run: notBuilt("run"),
  crew: notBuilt("crew"),
  chat: notBuilt("chat"),
  inbox: notBuilt("inbox"),
  history: notBuilt("history"),
  settings: notBuilt("settings"),
};
