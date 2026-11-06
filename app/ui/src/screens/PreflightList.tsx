import { StatusDot, Text } from "../../components/ui/base";
import type { Preflight } from "../ipc";

/** One row per check: a status dot, the name, the detail and, when it fails, the command that fixes it. */
export function PreflightList({ rows }: { rows: Preflight["rows"] }) {
  return (
    <ul className="v-home__checks">
      {rows.map((r) => (
        <li key={r.name}>
          <StatusDot status={r.ok ? "pass" : "fail"} />
          <span>{r.name}</span>
          <Text variant="body-sm" muted>{r.detail}</Text>
          {!r.ok && r.fix && <code className="v-home__fix">{r.fix}</code>}
        </li>
      ))}
    </ul>
  );
}
