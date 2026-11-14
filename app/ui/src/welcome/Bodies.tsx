// The open bodies of the three setup rows: the project folder, the agent and the engine. Each takes what it shows and what a
// click does; none of them calls the core itself, so the welcome owns every read and write.
import { useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { Button, Input, StatusDot, Text } from "../../components/ui/base";
import { S } from "../../strings";
import { api } from "../ipc";
import { OneClick, Steps } from "../screens/settings/SetupPanel";
import { ProviderIcon } from "../screens/ProviderIcon";
import { Problem } from "../screens/states";
import { openSignIn } from "../terminal/TerminalDock";
import type { SetupList, SetupRow } from "../setupContract";
import type { AgentInfo, Picked, Read, Repo, Repos, Tools } from "./model";
import { agentReady, agentSignedOut, reason } from "./model";
import type { CheckKind } from "./CheckRow";

const C = S.welcome;

/* ---- 1. Project folder --------------------------------------------------------------------------------------------- */

export interface FolderProps {
  repos: Repos;
  picked: Picked;
  /** Adding a project is a write: it waits for the person's pick. `others` are added too when the box is ticked. */
  onUse: (root: string, others: Repo[]) => void;
  busy: boolean;
  error: string;
  onLookAgain: () => void;
}

export function FolderBody({ repos, picked, onUse, busy, error, onLookAgain }: FolderProps) {
  const [also, setAlso] = useState(true);
  const [path, setPath] = useState("");
  const [pickFail, setPickFail] = useState(false);
  const choose = async () => {
    setPickFail(false);
    try {
      const p = await api.chooseFolder(C.folder.chooseTitle, picked?.root);
      if (p) onUse(p, []);
    } catch {
      setPickFail(true);
    }
  };
  const radios = useRef<HTMLDivElement>(null);
  const move = (e: KeyboardEvent<HTMLDivElement>) => {
    const dir = e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : 0;
    if (!dir) return;
    const all = Array.from(radios.current?.querySelectorAll<HTMLElement>('[role="radio"]') ?? []);
    const at = all.indexOf(e.target as HTMLElement);
    if (at < 0) return;
    e.preventDefault();
    all[(at + dir + all.length) % all.length]?.focus();
  };
  const found = repos.at === "found" ? repos : null;
  const others = found ? found.list.filter((r) => r.root !== picked?.root) : [];
  const picker = (
    <div className="v-wel__row">
      <Button size="sm" onClick={() => void choose()} disabled={busy} data-autofocus={!found || undefined}>{found ? C.folder.chooseAnother : C.folder.choose}</Button>
    </div>
  );
  const paste = (
    <form className="v-wel__paste" onSubmit={(e) => { e.preventDefault(); if (path.trim()) onUse(path.trim(), []); }}>
      <span className="v-wel__orpaste" aria-hidden="true">{C.folder.pastePath}</span>
      <div className="v-wel__path"><Input label={C.folder.pathLabel} placeholder={C.folder.pathPlaceholder} value={path} onChange={setPath} /></div>
      <Button size="sm" type="submit" disabled={busy || !path.trim()}>{C.folder.usePath}</Button>
    </form>
  );
  return (
    <>
      {repos.at === "loading" && <div role="status"><Text variant="body-sm">{C.folder.looking}</Text></div>}
      {found && (
        <>
          <Text variant="body-sm">{C.folder.foundHint(found.list.length)}</Text>
          <div className="v-wel__repos" role="radiogroup" aria-label={C.folder.repos} ref={radios} onKeyDown={move}>
            {found.list.map((r, i) => {
              const on = picked?.root === r.root;
              const tab = on || (!picked && i === 0);
              return (
                <button key={r.root} type="button" role="radio" aria-checked={on} tabIndex={tab ? 0 : -1} className="v-wel__repo" disabled={busy}
                  data-autofocus={tab || undefined} onClick={() => onUse(r.root, also ? found.list.filter((x) => x.root !== r.root) : [])}>
                  <span className="v-wel__radio" aria-hidden="true" />
                  <span className="v-wel__name">{r.name}</span>
                  <span className="v-wel__rpath">{r.root}</span>
                  {r.worktrees > 0 && <span className="v-wel__wt">{C.folder.worktrees(r.worktrees)}</span>}
                </button>
              );
            })}
          </div>
          {found.truncated && <Text variant="body-sm" muted>{C.folder.truncated}</Text>}
          <div className="v-wel__row v-wel__row--split">
            {(others.length > 0 || (!picked && found.list.length > 1)) && (
              <label className="v-wel__also">
                <input type="checkbox" checked={also} onChange={(e) => setAlso(e.target.checked)} />
                <span>{C.folder.alsoAdd(Math.max(found.list.length - 1, 1))}</span>
              </label>
            )}
            {picker}
          </div>
        </>
      )}
      {repos.at === "none" && (
        <>
          <Text variant="body-sm">{C.folder.noneBody}</Text>
          {picker}
          {paste}
          <Text variant="body-sm" muted>{C.folder.noneNeed}</Text>
        </>
      )}
      {repos.at === "failed" && (
        <>
          <div className="v-wel__notice" role="alert">
            <Text variant="label">{C.folder.failedTitle}</Text>
            <Text variant="body-sm">{C.folder.failedBody(repos.detail)}</Text>
          </div>
          <div className="v-wel__row">
            <Button size="sm" onClick={() => void choose()} disabled={busy} data-autofocus>{C.folder.choose}</Button>
            <Button size="sm" variant="ghost" onClick={onLookAgain}>{C.folder.again}</Button>
            <Text variant="body-sm" muted>{C.folder.failedHint}</Text>
          </div>
          {paste}
        </>
      )}
      {pickFail && <div role="alert"><Text variant="body-sm">{C.folder.pickFailed}</Text></div>}
      {error && <div role="alert"><Text variant="body-sm">{error}</Text></div>}
    </>
  );
}

/* ---- 2. Agent ------------------------------------------------------------------------------------------------------- */

interface AgentLine { kind: CheckKind; word: string }
function lineOf(a: AgentInfo): AgentLine {
  if (agentReady(a)) return { kind: "pass", word: C.agent.state.signed_in };
  if (agentSignedOut(a)) return { kind: "warn", word: C.agent.state.signed_out };
  if (a.row.state === "installed") return { kind: "todo", word: C.agent.state.cannot_tell };
  if (a.row.state === "broken") return { kind: "fail", word: S.setup.state.broken };
  return { kind: "todo", word: C.agent.state.not_installed };
}
const DOT = { pass: "pass", warn: "needs_approval", todo: "pending", fail: "fail" } as const;

export interface AgentProps {
  agents: Read<AgentInfo[]>;
  tools: Tools;
  onInstallTools: () => Promise<string>;
  reload: () => Promise<void>;
  retry: () => void;
  error: string;
}

export function AgentBody({ agents, tools, onInstallTools, reload, retry, error }: AgentProps) {
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [more, setMore] = useState<string | null>(null);
  const signedOut = Array.isArray(agents) && agents.some(agentSignedOut);
  const install = () => {
    setBusy(true);
    onInstallTools().then(setNote, (e) => setNote(reason(e) || C.agent.toolsFailed)).finally(() => setBusy(false));
  };
  return (
    <>
      {tools === "missing" && (
        <>
          <div className="v-wel__line">
            <StatusDot status="pending" label={S.welcome.word.todo} />
            <span className="v-wel__grow">{C.agent.toolsName}</span>
            <Button size="sm" onClick={install} busy={busy} data-autofocus>{C.agent.toolsInstall}</Button>
          </div>
          <Text variant="body-sm" muted>{C.agent.toolsHelp}</Text>
          <div role="status" className="v-wel__live">{note && <Text variant="body-sm">{note}</Text>}</div>
          <hr className="v-wel__sep" />
        </>
      )}
      {agents === "loading" && <Text variant="body-sm">{C.checking}</Text>}
      {agents === "failed" && <Problem title={C.agent.failed} detail={error} retry={retry} />}
      {Array.isArray(agents) && (
        <>
          <Text variant="body-sm" muted>{signedOut ? C.agent.signInHelp : agents.some(agentReady) ? "" : C.agent.needOne}</Text>
          <ul className="v-wel__agents">
            {agents.map((a) => {
              const l = lineOf(a);
              const installed = a.row.state === "installed";
              const off = !installed;
              return (
                <li key={a.row.id}>
                  <div className="v-wel__line">
                    <span className="v-wel__mark"><ProviderIcon family={a.row.id} /></span>
                    <span className="v-wel__aname">{a.row.name}</span>
                    <span className="v-wel__astate"><StatusDot status={DOT[l.kind]} label={l.word} /></span>
                    <span className="v-wel__grow v-wel__ver">{a.row.version}</span>
                    {agentSignedOut(a) && a.row.can_sign_in && (
                      <Button size="sm" data-autofocus onClick={() => openSignIn({ id: a.row.id, label: S.welcome.agent.signInTab(a.row.name) })}>{C.agent.signIn(a.row.name)}</Button>
                    )}
                    {off && (
                      <Button size="sm" variant={a.row.one_click ? "secondary" : "ghost"} aria-expanded={more === a.row.id}
                        data-autofocus={(!signedOut && tools !== "missing" && !agents.some(agentReady) && a.row.one_click) || undefined}
                        onClick={() => setMore(more === a.row.id ? null : a.row.id)}>
                        {a.row.one_click ? S.welcome.agent.install(a.row.name) : C.agent.yourself}
                      </Button>
                    )}
                  </div>
                  {off && more === a.row.id && (
                    <div className="v-wel__more">
                      {a.row.one_click && <OneClick row={a.row} reload={reload} opened={() => undefined} />}
                      {!a.row.one_click && <Steps row={a.row} />}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      )}
    </>
  );
}

/* ---- 3. Engine ------------------------------------------------------------------------------------------------------ */

export function EngineBody({ engine }: { engine: SetupList["engine"] | null }) {
  const [copy, setCopy] = useState("");
  if (engine?.installed) return <Text variant="body-sm" muted>{C.engine.readyBody}</Text>;
  if (!engine) return <Text variant="body-sm">{C.checking}</Text>;
  const doCopy = () => {
    const w = navigator.clipboard?.writeText(engine.command);
    if (!w) return setCopy(C.engine.copyFailed);
    w.then(() => setCopy(C.engine.copied), () => setCopy(C.engine.copyFailed));
  };
  return (
    <>
      <Text variant="body-sm">{C.engine.intro}</Text>
      <ol className="v-wel__steps">{C.engine.steps.map((s) => <li key={s}>{s}</li>)}</ol>
      <div className="v-wel__codewrap">
        <pre className="v-wel__code" tabIndex={0} aria-label={C.engine.lines}>{engine.command}</pre>
        <Button size="sm" onClick={doCopy} data-autofocus>{C.engine.copy}</Button>
      </div>
      <div role="status" className="v-wel__live">{copy && <Text variant="body-sm">{copy}</Text>}</div>
      <Text variant="body-sm" muted>{C.engine.changes}</Text>
    </>
  );
}

export type { SetupRow };
