// W2 part B: the Run view shows a core run's hash-chained log live. A notice that follows on is applied with no call; a
// return to the run reads only what it missed; a stop or a break is said, never hidden; text is never markup.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport, type RunLogEntry } from "../src/ipc";
import { Run } from "../src/screens/run/Run";
import { RunLog, forgetKeptRunLogs, what, who } from "../src/screens/run/RunLog";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const L = S.run.log;
afterEach(() => {
  cleanup();
  setTransport();
  forgetKeptRunLogs();
  window.location.hash = "";
});

const RUN = "20261113-2200";
const at = Date.UTC(2026, 10, 13, 22, 0, 0);
const entry = (seq: number, kind = "ticket_ended", body: unknown = { task: `t${seq}`, ticket: `k${seq % 4}`, outcome: "done" }, source: unknown = { worker: `k${seq % 4}` }): RunLogEntry =>
  ({ v: 1, seq, prev: `h${seq - 1}`, at_ms: at + seq * 1000, source, kind, body, hash: `h${seq}` });
const tick = () => act(async () => { for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0)); });

/** A core with a log of `n` entries; `since` answers like the core's `run_log_since`. */
function core(n: number, extra: Record<string, unknown> = {}) {
  let entries = Array.from({ length: n }, (_, i) => entry(i + 1));
  const ipc = fakeIpc({
    run_log_since: (a?: Record<string, unknown>) => {
      const after = Number(a?.afterSeq ?? 0);
      return { run_id: RUN, entries: entries.slice(after), head_seq: entries.length, broken: null, io_error: null };
    },
    ...extra,
  });
  setTransport(ipc.transport);
  const write = (k: number) => {
    const from = entries.length;
    entries = [...entries, ...Array.from({ length: k }, (_, i) => entry(from + i + 1))];
    return { run_id: RUN, seq: entries.length, after: from, entries: entries.slice(from), io_error: null, broken: null };
  };
  const calls = () => ipc.calls.filter((c) => c.cmd === "run_log_since");
  return { ipc, write, calls };
}

const rows = () => within(screen.getByRole("list", { name: L.entries })).getAllByRole("listitem").map((r) => r.textContent ?? "");

describe("what each entry says", () => {
  it("names who wrote it and what happened, for every kind the core writes", () => {
    expect([who("core"), who("verifier"), who("lead"), who("human"), who({ worker: "k3" })]).toEqual(["Vajra", "Verifier", "Lead", "You", "Worker k3"]);
    const say = (kind: string, body: unknown) => what(entry(1, kind, body));
    expect(say("run_started", { resumed_from: null })).toBe("Run started");
    expect(say("run_started", { resumed_from: "20261113-2100" })).toBe("Run resumed after 20261113-2100");
    expect(say("plan_accepted", { round: 1, tasks: ["t1", "t2"] })).toBe("Plan accepted for round 1: 2 tasks");
    expect(say("dispatched", { task: "t1", ticket: "k1", seat: "bhima" })).toBe("t1 handed to bhima");
    expect(say("integrated", { round: 2, ok: true, head: "abcdef123456", why: null })).toBe("Round 2 merged at abcdef1");
    expect(say("integrated", { round: 2, ok: false, head: null, why: "conflict in api.ts" })).toBe("Round 2 didn't merge: conflict in api.ts");
    expect(say("check_result", { round: 1, criterion: "AC-1", passed: false })).toBe("Check failed: AC-1");
    expect(say("round_verdict", { round: 1, verdict: "pass" })).toBe("Round 1 verdict: pass");
    expect(say("status_set", { status: "done", reason: "" })).toBe("Run status: done");
    expect(say("receipt", { scope: "run", model: "m", tokens: { fresh: 1000, cache_write: 0, cache_read: 500, output: 250, side: 3 }, usd_micros: 1_250_000, weight: null }))
      .toBe("Receipt for the run: m, 1,750 tokens, $1.25 at API prices");
    expect(say("receipt", { scope: "task:t1", model: "m", tokens: {}, usd_micros: 0, weight: null })).toBe("Receipt for t1: m, 0 tokens, $0.00 at API prices");
    expect(say("anchored", { seq: 40, hash: "x" })).toBe("Sent to the signed record up to entry 40");
    expect(say("recovered", { dropped_bytes: 17, at_seq: 9 })).toBe("After a crash, cut 17 bytes of a half-written line after entry 9");
    expect(say("something_new", {})).toBe("something_new");
    expect(say("question", null)).toBe("Question: ");
  });
});

describe("RunLog, live", () => {
  it("makes one call to start, then applies 4 workers' notices with none", async () => {
    const c = core(3);
    render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(3));
    for (let i = 0; i < 20; i++) act(() => c.ipc.emit("run_log", c.write(1 + (i % 4))));
    await tick();
    expect(screen.getByText(L.count(53))).toBeTruthy();
    expect(c.calls()).toHaveLength(1);
    expect(rows().at(-1)).toContain("t53 ended: done");
  });

  it("says a run has no log yet, and shows it once the core starts writing one, with no call", async () => {
    const ipc = fakeIpc({ run_log_since: () => Promise.reject({ error: "not_found", detail: "that run has no log" }) });
    setTransport(ipc.transport);
    render(<RunLog runId={RUN} />);
    expect(await screen.findByText(L.none)).toBeTruthy();
    act(() => ipc.emit("run_log", { run_id: RUN, seq: 1, after: 0, entries: [entry(1, "run_started", { resumed_from: null }, "core")], io_error: null, broken: null }));
    expect(await screen.findByText(L.count(1))).toBeTruthy();
    expect(screen.queryByText(L.none)).toBeNull();
    expect(rows()[0]).toContain("Vajra");
    expect(ipc.calls.filter((x) => x.cmd === "run_log_since")).toHaveLength(1);
  });

  it("coming back to a run reads only what it missed", async () => {
    const c = core(5);
    const first = render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(5));
    first.unmount();
    c.write(3); // written while the view was away; that notice reached no one
    render(<RunLog runId={RUN} />);
    expect(screen.getByText(L.count(5))).toBeTruthy(); // the kept view shows at once
    await screen.findByText(L.count(8));
    expect(c.calls().map((x) => x.args?.afterSeq)).toEqual([0, 5]);
  });

  it("coming back to a run whose log broke earlier on disk drops the kept entries from the break on", async () => {
    let broken = false;
    const all = Array.from({ length: 100 }, (_, i) => entry(i + 1));
    const ipc = fakeIpc({
      run_log_since: (a?: Record<string, unknown>) => {
        const after = Number(a?.afterSeq ?? 0);
        // Entry 50 was changed on disk: the core reads the good prefix, 1 to 49, and reports the break at 50.
        return broken
          ? { run_id: RUN, entries: all.slice(Math.min(after, 49), 49), head_seq: 49, broken: { seq: 50, why: "edited" }, io_error: null }
          : { run_id: RUN, entries: all.slice(after), head_seq: 100, broken: null, io_error: null };
      },
    });
    setTransport(ipc.transport);
    const first = render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(100));
    first.unmount();
    broken = true;
    render(<RunLog runId={RUN} />);
    expect(await screen.findByText(L.broken(50, L.why.edited))).toBeTruthy();
    expect(screen.getByText(L.count(49))).toBeTruthy();
    expect(screen.queryByText(L.count(100))).toBeNull();
    // Kept for the next visit as shown: 49 entries, so a third look shows no row past the break either.
    cleanup();
    render(<RunLog runId={RUN} />);
    expect(screen.getByText(L.count(49))).toBeTruthy();
  });

  it("coming back to a run whose log is shorter now drops the kept entries past its head", async () => {
    let head = 12;
    const all = Array.from({ length: 12 }, (_, i) => entry(i + 1));
    const ipc = fakeIpc({
      run_log_since: (a?: Record<string, unknown>) =>
        ({ run_id: RUN, entries: all.slice(Number(a?.afterSeq ?? 0), head), head_seq: head, broken: null, io_error: null }),
    });
    setTransport(ipc.transport);
    const first = render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(12));
    first.unmount();
    head = 7; // the file was cut back to 7 entries
    render(<RunLog runId={RUN} />);
    expect(await screen.findByText(L.count(7))).toBeTruthy();
  });

  it("says when the log stopped or its chain breaks, and keeps the entries it has", async () => {
    const c = core(2);
    render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(2));
    act(() => c.ipc.emit("run_log", { run_id: RUN, seq: 2, after: 2, entries: [], io_error: "the run log stopped at seq 2: disk full", broken: null }));
    const stopped = await screen.findByText(L.stopped);
    expect(stopped.closest("[role=alert]")?.textContent).toContain("disk full");
    act(() => c.ipc.emit("run_log", { run_id: RUN, seq: 2, after: 2, entries: [], io_error: "the run log stopped at seq 2: disk full", broken: { seq: 3, why: "edited" } }));
    expect(await screen.findByText(L.broken(3, L.why.edited))).toBeTruthy();
    expect(screen.getByText(L.count(2))).toBeTruthy();
  });

  it("shows an agent's text as text, never as markup", async () => {
    core(0);
    const ipc = fakeIpc({ run_log_since: { run_id: RUN, entries: [entry(1, "question", { id: "q1", text: "<img src=x onerror=alert(1)> ok?" }, "lead")], head_seq: 1, broken: null, io_error: null } });
    setTransport(ipc.transport);
    const { container } = render(<RunLog runId={RUN} />);
    expect(await screen.findByText("Question: <img src=x onerror=alert(1)> ok?")).toBeTruthy();
    expect(container.querySelector("img")).toBeNull();
  });

  it("checks the chain on request and shows its standing with the core's caveat", async () => {
    const caveat = "hash-chained; until the worker sandboxes land, an agent on this Mac could rewrite this log together with its anchors";
    let answer: unknown = { run_id: RUN, head_seq: 4, standing: "unconfirmed", verified: false, caveat, chain: { ok_to: 4, broken: null }, broken: null };
    core(4, { run_log_verify: () => (answer instanceof Error ? Promise.reject(answer) : answer) });
    render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(4));
    fireEvent.click(screen.getByRole("button", { name: L.check }));
    const status = await screen.findByText(L.standing.unconfirmed);
    expect(status.closest("[role=status]")?.textContent).toContain(`${L.checkedTo(4)} Hash-chained; until the worker sandboxes land`);
    answer = new Error("busy");
    fireEvent.click(screen.getByRole("button", { name: L.check }));
    expect(await screen.findByText(L.checkFailed)).toBeTruthy();
  });

  it("a signed check says where the signing stops and how many entries after it are only hash-chained", async () => {
    const signed = (signed_to: number, unsigned_tail: number) => ({
      run_id: RUN, head_seq: 50, standing: "signed", verified: true, caveat: "hash-chained", chain: { ok_to: 50, broken: null }, broken: null,
      signed: { signed_to, unsigned_tail, mismatch: null }, record_error: null, io_error: null, skipped_anchors: 0,
    });
    let answer = signed(40, 10);
    core(50, { run_log_verify: () => answer });
    render(<RunLog runId={RUN} />);
    await screen.findByText(L.count(50));
    fireEvent.click(screen.getByRole("button", { name: L.check }));
    const badge = await screen.findByText(L.standing.signed);
    const line = badge.closest("[role=status]")?.textContent ?? "";
    expect(line).toContain("Signed up to entry 40. The last 10 entries are hash-chained but not signed yet.");
    expect(line).not.toContain(L.checkedTo(50));
    answer = signed(50, 0);
    fireEvent.click(screen.getByRole("button", { name: L.check }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toContain("Signed up to entry 50. Hash-chained."));
    expect(L.signedTo(49, 1)).toBe("Signed up to entry 49. The last entry is hash-chained but not signed yet.");
  });
});

describe("Run view", () => {
  it("shows the run's log under its tasks, read by the run's id", async () => {
    window.location.hash = "#/run?slug=camp";
    const overview = { slug: "camp", run_id: RUN, stale: false, timeline: { phases: [], current: "Execute", status: "active" }, graph: { tasks: [], waves: 0 } };
    const c = core(2, { run_overview: overview, evidence_list: { slug: "camp", stale: false, rows: [] } });
    render(<Run />);
    expect(await screen.findByRole("heading", { name: L.title })).toBeTruthy();
    await screen.findByText(L.count(2));
    expect(c.calls()[0].args).toMatchObject({ runId: RUN, afterSeq: 0 });
  });
});
