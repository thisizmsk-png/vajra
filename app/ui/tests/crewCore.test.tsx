import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { parseCoreControls } from "../src/screens/crew/CoreWorkerPane";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";
import { Live, liveRun } from "./liveRun";

// jsdom has no modal dialog: model showModal/close the way browsers do.
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
});
afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

// The shape crew/tree.rs builds for a worker the core runs itself (16.7): `core:<ticket>`, with its controls and diff.
const STOP_NOTE = "Stop ends this worker at once; its uncommitted work stays in its worktree.";
const CHECK = "This worker checks the work on its own, so it can't take a message.";
const on = { message: { enabled: true, reason: null }, redirect: { enabled: true, reason: null }, stop: { enabled: true, reason: null, note: STOP_NOTE } };
const off = { message: { enabled: false, reason: CHECK }, redirect: { enabled: false, reason: CHECK }, stop: { enabled: true, reason: null, note: STOP_NOTE } };
const FINISHED = "This worker has finished.";
const ended = { message: { enabled: false, reason: FINISHED }, redirect: { enabled: false, reason: FINISHED }, stop: { enabled: false, reason: FINISHED, note: STOP_NOTE } };
const diff = { files_total: 3, files: ["src/a.rs", "src/b.rs"], added: 4, removed: 1, partial: false, uncounted: 0 };
const node = (id: string, kind: string, title: string, status: string, children: unknown[] = [], extra: object = {}) =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children, ...extra });
const tree = node("lead", "lead", "Lead", "in_progress", [
  node("run:r1", "run", "Run r1", "in_progress", [
    node("task:t1", "task", "Backend · t1", "in_progress", [node("core:r1.t1.r1", "round", "round 1", "in_progress", [], { controls: on, diff })]),
    node("verification:r1", "verification", "Verification r1", "unknown", [node("core:r1.verify.r1", "worker", "Verify", "in_progress", [], { controls: off })]),
  ]),
]);

async function open(pick: string, replies: Record<string, unknown> = {}) {
  window.location.hash = "#/crew?slug=camp&id=r1";
  // A live run: the crew view re-reads it on its own cadence.
  const f = fakeIpc({ ...liveRun(), crew: tree, ...replies });
  setTransport(f.transport);
  render(<Live><Crew /></Live>);
  const t = await screen.findByRole("tree", { name: S.crew.treeLabel });
  fireEvent.click(within(t).getByText(pick).closest("[role=treeitem]") as HTMLElement);
  return f;
}

describe("A core worker in the crew view (ADR-16 16.7, #151)", () => {
  it("selecting it opens its worker pane with its diff, not the generic facts pane", async () => {
    await open("round 1");
    const files = await screen.findByRole("list", { name: S.crew.core.changes });
    expect(within(files).getAllByRole("listitem").map((l) => l.textContent)).toEqual(["src/a.rs", "src/b.rs"]);
    expect(screen.getByText(S.crew.core.moreFiles(1))).toBeTruthy();
    expect(screen.queryByText(S.crew.kind)).toBeNull();
  });

  it("Message and Redirect send through the full core: reference", async () => {
    const f = await open("round 1", { control_message: { action_id: "a1", state: "queued" }, control_redirect: { action_id: "a2", state: "queued" } });
    fireEvent.change(await screen.findByLabelText(S.crew.core.text), { target: { value: "use the new API" } });
    fireEvent.click(screen.getByRole("button", { name: S.crew.core.message }));
    await screen.findByText(S.crew.core.sent);
    expect(f.calls.find((c) => c.cmd === "control_message")?.args).toEqual({ worker: "core:r1.t1.r1", text: "use the new API" });
    fireEvent.change(screen.getByLabelText(S.crew.core.text), { target: { value: "stop on b.rs" } });
    fireEvent.click(screen.getByRole("button", { name: S.crew.core.redirect }));
    await waitFor(() => expect(f.calls.find((c) => c.cmd === "control_redirect")?.args).toEqual({ worker: "core:r1.t1.r1", text: "stop on b.rs" }));
  });

  it("Stop asks first, names what it does, and stops the core: worker through the guarded action (a receipt, one action id)", async () => {
    const f = await open("round 1", { act_stop_worker: { receipt: { state: "done", reason: null, detail: null } } });
    fireEvent.click(await screen.findByRole("button", { name: S.crew.core.stop }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(STOP_NOTE)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "act_stop_worker")).toBe(false);
    fireEvent.click(within(dialog).getByRole("button", { name: S.crew.core.stop }));
    await screen.findByText(S.crew.core.stopped);
    const call = f.calls.filter((c) => c.cmd === "act_stop_worker");
    expect(call).toHaveLength(1);
    const args = call[0].args as { envelope: { action_id: string; project_id: string; expect_generation: number | null }; worker: string; reason: null };
    expect(args.worker).toBe("core:r1.t1.r1");
    expect(args.envelope.expect_generation).toBe(1);
    expect(args.envelope.action_id).toMatch(/\S/);
    expect(f.calls.some((c) => c.cmd === "control_stop")).toBe(false);
  });

  it("a stop the core refuses says why, from its receipt", async () => {
    await open("round 1", { act_stop_worker: { receipt: { state: "rejected", reason: "stale_generation", detail: null } } });
    fireEvent.click(await screen.findByRole("button", { name: S.crew.core.stop }));
    fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: S.crew.core.stop }));
    expect(await screen.findByText(S.crew.core.notStopped("stale_generation"))).toBeTruthy();
  });

  it("a worker its harness can't message shows Message and Redirect off with the core's reason, and sends nothing", async () => {
    const f = await open("Verify");
    fireEvent.change(await screen.findByLabelText(S.crew.core.text), { target: { value: "hi" } });
    const msg = screen.getByRole("button", { name: S.crew.core.message });
    expect(msg.getAttribute("aria-disabled")).toBe("true");
    expect(screen.getByRole("button", { name: S.crew.core.redirect }).getAttribute("aria-disabled")).toBe("true");
    const why = document.getElementById(msg.getAttribute("aria-describedby") ?? "");
    expect(why?.textContent).toBe(CHECK);
    fireEvent.click(msg);
    expect(f.calls.some((c) => c.cmd === "control_message")).toBe(false);
    expect(screen.getByText(S.crew.core.noDiff)).toBeTruthy();
  });

  it("when its ticket ends, the refreshed pane offers no Message, Redirect or Stop, and says it finished", async () => {
    let live = true;
    const crew = () => node("lead", "lead", "Lead", "in_progress", [
      node("run:r1", "run", "Run r1", "in_progress", [
        node("task:t1", "task", "Backend · t1", live ? "in_progress" : "done", [node("core:r1.t1.r1", "round", "round 1", live ? "in_progress" : "done", [], { controls: live ? on : ended, diff })]),
      ]),
    ]);
    await open("round 1", { crew });
    const stop = await screen.findByRole("button", { name: S.crew.core.stop });
    expect(stop.getAttribute("aria-disabled")).toBeNull();
    live = false;
    // The crew view re-reads on its own cadence.
    await waitFor(() => expect(screen.getByRole("button", { name: S.crew.core.stop }).getAttribute("aria-disabled")).toBe("true"), { timeout: 4000 });
    for (const name of [S.crew.core.message, S.crew.core.redirect, S.crew.core.stop]) {
      const b = screen.getByRole("button", { name });
      expect(b.getAttribute("aria-disabled")).toBe("true");
      expect(document.getElementById(b.getAttribute("aria-describedby") ?? "")?.textContent).toBe(FINISHED);
    }
    // The Stop note is not shown beside a Stop that can't be used (only the closed confirm dialog still holds it).
    expect(screen.queryAllByText(STOP_NOTE).filter((el) => !el.closest("dialog"))).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: S.crew.core.stop }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("counts core workers among the run's workers", async () => {
    await open("Run r1");
    expect(await screen.findByText(S.crew.workersDone(0, 2))).toBeTruthy();
  });

  it("reads malformed controls as none, so nothing is offered", () => {
    expect(parseCoreControls({ message: { enabled: "yes" } })).toBeNull();
    expect(parseCoreControls(on)).toEqual(on);
  });
});
