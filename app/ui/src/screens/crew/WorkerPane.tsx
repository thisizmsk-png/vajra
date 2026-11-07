import { useEffect, useState } from "react";
import { Badge, Button, Section, Text } from "../../../components/ui/base";
import { DiffView, Tabs } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText } from "../../ipc";
import type { CrewNodeDto, Json, WorktreeInfo } from "../../ipc";
import { StreamTab } from "./StreamTab";

type TabId = "stream" | "diff" | "task" | "worktree";

/** The task graph id (`<run>.<tid>`) of a worker whose crew-tree parent is the task group `task:<tid>`; null for any other worker. */
export function taskGraphId(runId: string, taskLocalId: string | null): string | null {
  return taskLocalId ? `${runId}.${taskLocalId}` : null;
}

const asList = (v: unknown): string[] => (Array.isArray(v) ? v.map((x) => (typeof x === "string" ? x : JSON.stringify(x))) : []);

function Copy({ what, text }: { what: string; text: string }) {
  // The clipboard only: nothing here opens, runs or reveals the path.
  return <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(text)}>{S.crew.copy(what)}</Button>;
}

function DiffTab({ info }: { info: WorktreeInfo }) {
  const [r, setR] = useState<{ diff: string; code: number | null; truncated: boolean; timed_out: boolean; untracked?: string[]; untracked_truncated?: boolean } | { err: string } | null>(null);
  useEffect(() => {
    let live = true;
    if (!info.base) { setR({ err: S.crew.noBase }); return; }
    api.diff(info.worktree, info.base).then((d) => live && setR(d), (e) => live && setR({ err: errorText(e) }));
    return () => { live = false; };
  }, [info]);
  if (!r) return <Text variant="body-sm" muted>{S.crew.loading}</Text>;
  if ("err" in r) return <div role="alert"><Text variant="body-sm">{r.err}</Text></div>;
  if (r.timed_out) return <div role="alert"><Text variant="body-sm">{S.crew.diffTimedOut}</Text></div>;
  if (r.code !== 0) return <div role="alert"><Text variant="body-sm">{S.crew.diffFailed(r.code)}</Text></div>;
  return (
    <>
      {r.truncated && <Text variant="body-sm">{S.crew.diffTruncated}</Text>}
      {r.diff.trim() === "" && !r.untracked?.length ? <Text variant="body-sm" muted>{S.crew.noChanges}</Text> : r.diff.trim() !== "" && <DiffView diff={r.diff} />}
      {!!r.untracked?.length && (
        <div role="group" aria-label={S.crew.untracked}>
          <Text variant="body-sm">{S.crew.untracked}</Text>
          <ul>{r.untracked.map((f) => (<li key={f}><code>{f}</code></li>))}</ul>
          {r.untracked_truncated && <Text variant="body-sm" muted>{S.crew.untrackedMore}</Text>}
        </div>
      )}
    </>
  );
}

function TaskTab({ slug, taskId }: { slug: string; taskId: string | null }) {
  const [t, setT] = useState<Json | "none" | { err: string } | null>(null);
  useEffect(() => {
    let live = true;
    if (!taskId) { setT("none"); return; }
    api.task(slug, taskId).then(
      (r) => live && setT((r.task as Json) ?? "none"),
      (e) => live && setT(typeof e === "object" && e && (e as { error?: string }).error === "task_not_found" ? "none" : { err: errorText(e) }),
    );
    return () => { live = false; };
  }, [slug, taskId]);
  if (!t) return <Text variant="body-sm" muted>{S.crew.loading}</Text>;
  if (t === "none") return <Text variant="body-sm" muted>{S.crew.noTask}</Text>;
  if ("err" in t) return <div role="alert"><Text variant="body-sm">{String(t.err)}</Text></div>;
  const report = t.report as { status?: string; summary?: string } | undefined;
  const rows: [string, string][] = [
    [S.crew.owner, String(t.owner_agent ?? "")],
    [S.crew.deps, asList(t.deps).join(", ")],
    [S.crew.files, asList(t.files).join(", ")],
    [S.crew.acceptance, asList(t.acceptance).join("; ")],
    [S.crew.attempts, String(t.attempts ?? "")],
    [S.crew.lastReport, report ? [report.status, report.summary].filter(Boolean).join(": ") : ""],
  ];
  return (
    <dl className="v-crew__facts" role="group" aria-label={S.crew.taskFacts}>
      <dt>{S.crew.taskTitle}</dt><dd>{String(t.title ?? "")}</dd>
      {rows.map(([k, v]) => (<><dt key={`k${k}`}>{k}</dt><dd key={`v${k}`}>{v}</dd></>))}
    </dl>
  );
}

function WorktreeTab({ info }: { info: WorktreeInfo | null | undefined }) {
  if (info === undefined) return <Text variant="body-sm" muted>{S.crew.loading}</Text>;
  if (info === null) return <Text variant="body-sm" muted>{S.crew.noWorktree}</Text>;
  return (
    <dl className="v-crew__facts" role="group" aria-label={S.crew.worktreeFacts}>
      <dt>{S.crew.worktreePath}</dt><dd>{info.worktree} <Copy what={S.crew.worktreePath} text={info.worktree} /></dd>
      <dt>{S.crew.branch}</dt><dd>{info.branch ?? ""} {info.branch && <Copy what={S.crew.branch} text={info.branch} />}</dd>
      <dt>{S.crew.baseCommit}</dt><dd>{info.base ?? ""} {info.base && <Copy what={S.crew.baseCommit} text={info.base} />}</dd>
    </dl>
  );
}

/** One worker: header (task, worktree, branch) and the Stream / Diff / Task / Worktree tabs. */
export function WorkerPane({ node, slug, runId, taskLocalId, status }: { node: CrewNodeDto; slug: string; runId: string; taskLocalId: string | null; status: string }) {
  const agent = node.id.slice("agent:".length);
  const [tab, setTab] = useState<TabId>("stream");
  const [info, setInfo] = useState<WorktreeInfo | null | undefined>(undefined);
  const [task, setTask] = useState<Json | null>(null);
  const taskId = taskGraphId(runId, taskLocalId);

  useEffect(() => {
    let live = true;
    api.worktreeInfo(agent).then((i) => live && setInfo(i), () => live && setInfo(null));
    return () => { live = false; };
  }, [agent]);
  useEffect(() => {
    let live = true;
    if (taskId) api.task(slug, taskId).then((r) => live && setTask((r.task as Json) ?? null), () => undefined);
    return () => { live = false; };
  }, [slug, taskId]);

  return (
    <>
      <Section title={S.crew.details}>
        <dl className="v-crew__facts" role="group" aria-label={S.crew.header}>
          <dt>{S.crew.role}</dt><dd>{node.role ?? node.title}</dd>
          {node.engine_id && (<><dt>{S.crew.engineId}</dt><dd>{node.engine_id}</dd></>)}
          <dt>{S.crew.status}</dt><dd><Badge>{status}</Badge></dd>
          <dt>{S.crew.task}</dt><dd>{taskId ? `${taskId}${task?.title ? ` ${String(task.title)}` : ""}` : S.crew.noTask}</dd>
          {info && (<>
            <dt>{S.crew.worktreePath}</dt><dd>{info.worktree} <Copy what={S.crew.worktreePath} text={info.worktree} /></dd>
            <dt>{S.crew.branch}</dt><dd>{info.branch ?? ""} {info.branch && <Copy what={S.crew.branch} text={info.branch} />}</dd>
          </>)}
        </dl>
      </Section>
      <Tabs
        label={S.crew.tabsLabel}
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "stream", label: S.crew.tabs.stream, panel: <StreamTab stream={`w-${agent}`} /> },
          { id: "diff", label: S.crew.tabs.diff, panel: info ? <DiffTab info={info} /> : <Text variant="body-sm" muted>{info === null ? S.crew.noWorktree : S.crew.loading}</Text> },
          { id: "task", label: S.crew.tabs.task, panel: <TaskTab slug={slug} taskId={taskId} /> },
          { id: "worktree", label: S.crew.tabs.worktree, panel: <WorktreeTab info={info} /> },
        ]}
      />
    </>
  );
}
