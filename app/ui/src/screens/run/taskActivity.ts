// What each working task of a run is doing now, for the Review page's task rows: its worker's newest line. One crew read
// maps a task to its latest worker; then a few kilobytes of each working worker's stream. Read only while the run is live
// and the window shows, so a finished run costs nothing.
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../ipc";
import type { CrewNodeDto } from "../../ipc";
import { usePoll } from "../../projects/poll";
import { LIVENESS_MS, isWorkerId } from "../crew/crewLive";
import { chatItems, lastLine } from "../crew/chat";

const ROWS = 12;
const BYTES = 8 * 1024;
/** Working tasks read at once; a run rarely has more working, and the rest keep their status alone. */
export const MAX_TASKS = 16;

/** Each task group's latest worker that writes a stream (`agent:<id>`), by the task's run-local id. */
export function latestWorkers(root: CrewNodeDto): Map<string, string> {
  const out = new Map<string, string>();
  const walk = (n: CrewNodeDto) => {
    if (n.id.startsWith("task:")) {
      const w = n.children.filter((c) => isWorkerId(c.id)).pop();
      if (w?.id.startsWith("agent:")) out.set(n.id.slice("task:".length), w.id.slice("agent:".length));
    }
    n.children.forEach(walk);
  };
  walk(root);
  return out;
}

/** The newest line of each task in `working` (run-local ids), while `live`. */
export function useTaskActivity(slug: string, runId: string, working: readonly string[], live: boolean): ReadonlyMap<string, string> {
  const [lines, setLines] = useState<ReadonlyMap<string, string>>(new Map());
  const key = working.slice(0, MAX_TASKS).join("\n");
  const gen = useRef(0);
  // A new run, or the run no longer live: drop what was read, so a line never outlives the work it described.
  useEffect(() => { gen.current++; setLines(new Map()); }, [slug, runId, live]);
  const read = useCallback(() => {
    // Hidden window: nothing is read; usePoll reads again as soon as the window shows.
    if (!key || !live || !runId || document.hidden) return;
    const mine = gen.current;
    const want = key.split("\n");
    void api.crew(slug, runId).then(async (root) => {
      const by = latestWorkers(root);
      const got = await Promise.all(want.map((t) => {
        const agent = by.get(t);
        return agent ? api.streamWindow(`w-${agent}`, null, ROWS, BYTES).then((w) => [t, lastLine(chatItems(w.lines))] as [string, string], () => null) : null;
      }));
      if (mine === gen.current) setLines(new Map(got.filter((g): g is [string, string] => !!g && g[1] !== "")));
    }, () => undefined);
  }, [slug, runId, key, live]);
  useEffect(read, [read]);
  usePoll(read, LIVENESS_MS, live && !!key);
  return lines;
}
