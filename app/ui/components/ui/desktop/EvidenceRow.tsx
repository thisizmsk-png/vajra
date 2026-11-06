import { StatusDot } from "../base/StatusDot";

/** One acceptance criterion with its check: id (the row header), command, exit code, evidence file and commit. */
export function EvidenceRow({
  id,
  state,
  command,
  exitCode,
  evidenceFile,
  commit,
}: {
  id: string;
  state: "pass" | "fail" | "unknown";
  command: string;
  exitCode?: number;
  evidenceFile?: string;
  commit?: string;
}) {
  return (
    <tr className="v-evidence">
      <th scope="row" className="v-text v-text--label">{id}</th>
      <td><StatusDot status={state} /></td>
      <td className="v-text v-text--mono">{command}</td>
      <td className="v-num">{exitCode ?? ""}</td>
      <td className="v-text v-text--mono">{evidenceFile ?? ""}</td>
      <td className="v-text v-text--mono">{commit ? commit.slice(0, 7) : ""}</td>
    </tr>
  );
}
