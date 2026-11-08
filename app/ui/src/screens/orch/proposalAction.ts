// Start or Dismiss a run proposal, and what the UI says happened (ADR-15 §3.2 states). The one place to swap in the
// FG-11 envelope (action_id + expect generation, returning a receipt) once FG-02b takes it; until then this reads
// FG-02b's `start_proposal` / `dismiss_proposal` answers and errors the same way:
//   - an answer       -> done (started / dismissed)
//   - a core refusal  -> refused, with its reason; `proposal_changed` says what to do
//   - no answer       -> unknown: check before retrying (it may have started). Never retried by itself.
import { S } from "../../../strings";
import { api, errorText, isIpcError } from "../../ipc";
import type { InboxItem } from "../../ipc";

export type Outcome = { kind: "pending" | "done" | "refused" | "unknown"; text: string };

export async function proposalAction(what: "start" | "dismiss", item: InboxItem): Promise<Outcome> {
  try {
    if (what === "start") {
      if (!item.proposal_sha256) return { kind: "refused", text: S.dg.receipt.changed };
      await api.startProposal(item.id, item.proposal_sha256);
      return { kind: "done", text: S.dg.receipt.started };
    }
    await api.dismissProposal(item.id);
    return { kind: "done", text: S.dg.receipt.dismissed };
  } catch (e) {
    if (!isIpcError(e)) return { kind: "unknown", text: S.dg.receipt.unknown };
    if (e.error === "proposal_changed") return { kind: "refused", text: S.dg.receipt.changed };
    return { kind: "refused", text: S.dg.receipt.refused(errorText(e)) };
  }
}
