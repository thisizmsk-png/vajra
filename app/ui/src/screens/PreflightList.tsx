import { useState } from "react";
import { Button, StatusDot, Text } from "../../components/ui/base";
import { api, errorText } from "../ipc";
import type { Preflight, PreflightRow } from "../ipc";
import { S } from "../../strings";

/** One row per check: a status dot, the name, the detail and, when it fails, the command or the one-click action that fixes it. */
export function PreflightList({ rows }: { rows: Preflight["rows"] }) {
  return (
    <ul className="v-home__checks">
      {rows.map((r) => (
        <li key={r.name}>
          <StatusDot status={r.ok ? "pass" : "fail"} />
          <span>{Object.prototype.hasOwnProperty.call(S.home.checkNames, r.name) ? S.home.checkNames[r.name] : r.name}</span>
          <Text variant="body-sm" muted>{r.detail}</Text>
          {!r.ok && r.fix && <code className="v-home__fix">{r.fix}</code>}
          {!r.ok && r.action && <RowAction row={r} />}
        </li>
      ))}
    </ul>
  );
}

/** The action runs only on the human's click (Apple's installer is never opened on its own). */
function RowAction({ row }: { row: PreflightRow }) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const run = () => {
    setBusy(true);
    api.installDevTools().then(() => setNote(row.action_note ?? ""), (e) => setNote(errorText(e))).finally(() => setBusy(false));
  };
  return (
    <span className="v-home__action">
      <Button size="sm" busy={busy} onClick={run}>{row.action_label ?? row.action}</Button>
      <span role="status">{note}</span>
    </span>
  );
}
