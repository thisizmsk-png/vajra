import type { ReactNode } from "react";
import { Text } from "./Text";

/** The title row of a screen: one h1, an optional one-line lede, and an optional action at the end. */
export function PageHeader({ title, lede, action }: { title: string; lede?: string; action?: ReactNode }) {
  return (
    <header className="v-pagehead">
      <div className="v-pagehead__text">
        <Text as="h1" variant="headline-lg">{title}</Text>
        {lede && <Text variant="body" muted>{lede}</Text>}
      </div>
      {action}
    </header>
  );
}
