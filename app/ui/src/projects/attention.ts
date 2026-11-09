// What an arrival says out loud (a screen-reader announcement, and later an OS notification): the kind and the project
// only. An item's title and body are untrusted and may hold secrets, so they never leave the inbox view (FG-07).
import { S } from "../../strings";

export type AttentionKind = "plan" | "question" | "task" | "failed";

export function attentionText(kind: AttentionKind, project: string): string {
  return S.inbox.attention[kind](project);
}
