import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Run } from "../src/screens/run/Run";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const row = (task: string, criterion: string, status: string, extra: object = {}) =>
  ({ task, title: `Task ${task}`, owner: "bhima", criterion, status, head: null, path: null, commands: null, report: null, ...extra });
const list = (rows: unknown[]) => ({ slug: "camp", stale: false, rows });
const open = async (rows: unknown[]) => {
  window.location.hash = "#/run?slug=camp";
  const f = fakeIpc({ evidence_list: list(rows) });
  setTransport(f.transport);
  render(<Run />);
  await waitFor(() => expect(f.calls.some((c) => c.cmd === "evidence_list")).toBe(true));
  if (rows.length) await screen.findAllByRole("listitem");
  else await screen.findByText(S.run.none);
  return f;
};
const order = () => screen.getAllByRole("listitem").map((e) => e.textContent ?? "");

describe("Run view, Evidence (S4a, POS-1.1, US-4.3)", () => {
  it("is the run's default view and asks the core for the campaign's criteria", async () => {
    const f = await open([row("t1", "ac-1", "pass")]);
    expect(f.calls.find((c) => c.cmd === "evidence_list")?.args).toEqual({ slug: "camp" });
    expect(screen.getByRole("heading", { name: S.run.evidence })).toBeTruthy();
  });

  it("lists failing criteria first, then unverified, not run, and passing last", async () => {
    await open([row("t1", "ac-pass", "pass"), row("t2", "ac-nr", "not_run"), row("t3", "ac-un", "unverified"), row("t4", "ac-fail", "fail")]);
    const o = order();
    expect(o[0]).toContain("ac-fail");
    expect(o[1]).toContain("ac-un");
    expect(o[2]).toContain("ac-nr");
    expect(o[3]).toContain("ac-pass");
  });

  it("states every status as text", async () => {
    await open([row("t1", "a", "pass"), row("t2", "b", "fail"), row("t3", "c", "not_run"), row("t4", "d", "unverified")]);
    const text = order().join(" ");
    for (const s of [S.run.status.pass, S.run.status.fail, S.run.status.not_run, S.run.status.unverified]) expect(text).toContain(s);
  });

  it("never shows a green summary while any criterion is not a pass", async () => {
    await open([row("t1", "a", "pass"), row("t2", "b", "unverified")]);
    expect(screen.queryByText(S.run.allPass)).toBeNull();
    expect(screen.getByText(S.run.summary(1, 2)).closest(".v-badge--success")).toBeNull();
  });

  it("shows the all-pass summary only when every criterion passes", async () => {
    await open([row("t1", "a", "pass"), row("t2", "b", "pass")]);
    expect(screen.getByText(S.run.allPass).closest(".v-badge--success")).toBeTruthy();
  });

  it("a campaign with no tasks has nothing to pass and no green badge", async () => {
    await open([]);
    expect(screen.queryByText(S.run.allPass)).toBeNull();
    expect(screen.getByText(S.run.none)).toBeTruthy();
  });

  it("selecting a row shows its proof: commit, evidence file, command count, report", async () => {
    await open([row("t2", "ac-2", "unverified", { head: "a".repeat(40), path: "/camp/evidence/e.json", commands: 3, report: "tests red" })]);
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    const d = await screen.findByRole("group", { name: S.run.detail });
    for (const t of ["aaaaaaa", "/camp/evidence/e.json", "3", "tests red", "t2"]) expect(d.textContent).toContain(t);
  });

  it("a criterion without evidence says no evidence, never pass", async () => {
    await open([row("t1", "ac-1", "not_run")]);
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    expect((await screen.findByRole("group", { name: S.run.detail })).textContent).toContain(S.run.noEvidence);
  });

  it("renders only the rows in view for 2,000 criteria", async () => {
    await open(Array.from({ length: 2000 }, (_, i) => row(`t${i}`, `ac-${i}`, i === 0 ? "fail" : "pass")));
    expect(screen.getAllByRole("listitem").length).toBeLessThan(60);
    expect(screen.getByText(S.run.summary(1999, 2000))).toBeTruthy();
  });

  it("shows a read error as an alert and keeps the page", async () => {
    window.location.hash = "#/run?slug=camp";
    setTransport(fakeIpc({ evidence_list: new Error("x") }).transport);
    render(<Run />);
    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("asks for a campaign when none is chosen", async () => {
    window.location.hash = "#/run";
    const f = fakeIpc({});
    setTransport(f.transport);
    render(<Run />);
    expect(await screen.findByText(S.run.pick)).toBeTruthy();
    await waitFor(() => expect(f.calls).toHaveLength(0));
  });
});
