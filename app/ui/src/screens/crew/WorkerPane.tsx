import { useEffect, useState } from "react";
import { AlertDialog, Button, Section, StatusDot, Text } from "../../../components/ui/base";
import type { StatusKey } from "../../../strings";
import { DiffView, Tabs } from "../../../components/ui/desktop";
import { S } from "../../../strings";
import { api, errorText, newActionId } from "../../ipc";
import { unsavedText, useCloseout } from "../../projects/closeout";
import { useCurrentProject } from "../../projects/current";
import type { CrewNodeDto, Json, WorktreeInfo } from "../../ipc";
import { StreamTab } from "./StreamTab";
import { DiffCount, ProviderLine } from "../ProviderLine";
import { parseWorkerDiff, parseWorkerProvider } from "../../workerProvider";

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

type TaskState = Json | "none" | { err: string } | null;

function TaskTab({ t }: { t: TaskState }) {
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

const STATUS_KEYS = ["pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed", "unknown"] as const;
const WORKTREE_RETRY_MS = 3000;
/** Same cadence as the Crew screen's own refresh. */
const TASK_POLL_MS = 2000;

/** One worker: header (task, worktree, branch) and the Stream / Diff / Task / Worktree tabs. */
export function WorkerPane({ node, slug, runId, taskLocalId, status }: { node: CrewNodeDto; slug: string; runId: string; taskLocalId: string | null; status: string }) {
  const agent = node.id.slice("agent:".length);
  const wp = parseWorkerProvider(node.provider);
  const wd = parseWorkerDiff(node.diff);
  const [tab, setTab] = useState<TabId>("stream");
  const [info, setInfo] = useState<WorktreeInfo | null | undefined>(undefined);
  const [task, setTask] = useState<TaskState>(null);
  const taskId = taskGraphId(runId, taskLocalId);

  // A worker can exist before its transcript names a cwd, so the first answer may be "no crew worktree". Keep asking
  // while the worker is live; a worker that finished (or never had a crew worktree) is asked once more per status change.
  const finished = status === "done" || status === "failed";
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = () => api.worktreeInfo(agent).then(
      (i) => live && setInfo(i),
      () => {
        if (!live) return;
        setInfo(null);
        if (!finished) timer = setTimeout(ask, WORKTREE_RETRY_MS);
      },
    );
    void ask();
    return () => { live = false; clearTimeout(timer); };
  }, [agent, finished]);
  // One task read shared by the header and the Task tab, repeated on the crew's own cadence so attempts and the last
  // report follow tasks.json while this worker stays selected.
  useEffect(() => {
    let live = true;
    if (!taskId) { setTask("none"); return; }
    const read = () => api.task(slug, taskId).then(
      (r) => live && setTask((r.task as Json) ?? "none"),
      (e) => live && setTask(typeof e === "object" && e && (e as { error?: string }).error === "task_not_found" ? "none" : { err: errorText(e) }),
    );
    void read();
    const poll = setInterval(() => { if (!document.hidden) void read(); }, TASK_POLL_MS);
    return () => { live = false; clearInterval(poll); };
  }, [slug, taskId]);
  // A finished worker with work that exists only in its worktree: shown with the exact counts, and the one way to close it.
  const closeout = useCloseout(slug, runId);
  const { project } = useCurrentProject();
  const co = closeout.rows.get(agent);
  const [confirming, setConfirming] = useState(false);
  const [closeMsg, setCloseMsg] = useState("");
  const closeAnyway = () => {
    setConfirming(false);
    if (!co || !project) return;
    const confirmed = { changedFiles: co.changed_files ?? 0, unpushedCommits: co.unpushed_commits ?? 0, busyTerminals: co.busy_terminals ?? 0, noUpstream: co.no_upstream ?? false };
    void api.actCloseWorker({ action_id: newActionId(), project_id: project.id, expect_generation: null }, agent, confirmed).then(
      (r) => {
        const e = r.error as { error?: string; changed_files?: number; unpushed_commits?: number; no_upstream?: boolean; busy_terminals?: number } | undefined;
        setCloseMsg(r.receipt?.state === "done" && !e ? "" : e?.error === "unsaved_work_changed" ? S.closeout.changed(unsavedText(e)) : S.closeout.notDone);
      },
      () => setCloseMsg(S.closeout.notDone),
    ).finally(closeout.reload);
  };
  const taskObj = task && task !== "none" && !("err" in task) ? task : null;

  const shape = (STATUS_KEYS as readonly string[]).includes(node.status) ? (node.status as StatusKey) : "unknown";

  return (
    <>
      <section className="v-crew__pane" aria-label={S.crew.header}>
        <div className="v-crew__panehead">
          <Text as="h2" variant="headline-md">{node.role ?? node.title}</Text>
          <StatusDot status={shape} label={status} />
          {wp && <ProviderLine p={wp} live />}
          {wd && <DiffCount d={wd} />}
          <span className="v-crew__task">{taskId ? `${taskId}${taskObj?.title ? ` ${String(taskObj.title)}` : ""}` : S.crew.noTask}</span>
        </div>
        {co?.state === "closed" && <p className="v-closeout" role="status">{S.closeout.closedHint}</p>}
        {(co?.state === "unsaved_work" || co?.state === "terminal_busy") && (
          <div className="v-closeout v-closeout--warn" role="group" aria-label={S.closeout.unsaved}>
            <p>{S.closeout.hasUnsaved(unsavedText(co))}</p>
            <Button size="sm" variant="danger" onClick={() => setConfirming(true)}>{S.closeout.closeAnyway}</Button>
          </div>
        )}
        {closeMsg && <p className="v-closeout" role="alert">{closeMsg}</p>}
        <AlertDialog open={confirming} title={S.closeout.confirmTitle} confirmLabel={S.closeout.confirm} destructive onConfirm={closeAnyway} onCancel={() => setConfirming(false)}>
          <p>{co ? S.closeout.confirmBody(unsavedText(co)) : ""}</p>
        </AlertDialog>
        <Tabs
          label={S.crew.tabsLabel}
          value={tab}
          onChange={setTab}
          tabs={[
            { id: "stream", label: S.crew.tabs.stream, panel: <StreamTab stream={`w-${agent}`} /> },
            { id: "diff", label: S.crew.tabs.diff, panel: info ? <DiffTab info={info} /> : <Text variant="body-sm" muted>{info === null ? S.crew.noWorktree : S.crew.loading}</Text> },
            { id: "task", label: S.crew.tabs.task, panel: <TaskTab t={task} /> },
            { id: "worktree", label: S.crew.tabs.worktree, panel: <WorktreeTab info={info} /> },
          ]}
        />
      </section>
      <aside className="v-crew__aside" aria-label={S.crew.details}>
        <Section title={S.crew.details}>
          <dl className="v-crew__facts" role="group" aria-label={S.crew.header}>
            <dt>{S.crew.role}</dt><dd>{node.role ?? node.title}</dd>
            {node.engine_id && (<><dt>{S.crew.engineId}</dt><dd>{node.engine_id}</dd></>)}
            <dt>{S.crew.task}</dt><dd>{taskId ? `${taskId}${taskObj?.title ? ` ${String(taskObj.title)}` : ""}` : S.crew.noTask}</dd>
            {info && (<>
              <dt>{S.crew.worktreePath}</dt><dd>{info.worktree} <Copy what={S.crew.worktreePath} text={info.worktree} /></dd>
              <dt>{S.crew.branch}</dt><dd>{info.branch ?? ""} {info.branch && <Copy what={S.crew.branch} text={info.branch} />}</dd>
            </>)}
          </dl>
        </Section>
      </aside>
    </>
  );
}
