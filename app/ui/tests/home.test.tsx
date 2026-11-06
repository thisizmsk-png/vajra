import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Home } from "../src/screens/home/Home";
import { validate } from "../src/screens/home/validate";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const rows = (ok: boolean) => ({
  rows: [
    { name: "claude", ok: true, detail: "2.1.0", fix: "" },
    { name: "hooks", ok, detail: ok ? "registered" : "not registered", fix: ok ? "" : "bash install.sh" },
  ],
  start_enabled: ok,
});
const handle = { stream: "s1", session_id: "u", run_id: "20261106-1500", campaign: "deliver-20261106-1500" };
const camp = (o = {}) => ({ slug: "c1", goal: "ship it", status: "active", passing: 2, total: 5, ...o });

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

async function open(replies: Record<string, unknown>) {
  const f = fakeIpc(replies);
  setTransport(f.transport);
  render(<Home />);
  await waitFor(() => expect(f.calls.map((c) => c.cmd)).toContain("list_campaigns"));
  return f;
}
const fill = (label: string, v: string) => fireEvent.change(screen.getByLabelText(label), { target: { value: v } });

describe("validate", () => {
  const ok = { repo: "/r", ask: "do", base: "main", rounds: "2", maxInFlight: "4" };
  it("accepts a good draft and names each bad field", () => {
    expect(validate(ok)).toEqual({});
    expect(Object.keys(validate({ repo: " ", ask: "", base: "", rounds: "4", maxInFlight: "17" })).sort()).toEqual(["ask", "base", "maxInFlight", "repo", "rounds"]);
    expect(validate({ ...ok, maxInFlight: "1.5" }).maxInFlight).toBeTruthy();
    expect(validate({ ...ok, rounds: "0" }).rounds).toBeTruthy();
  });
});

describe("Home", () => {
  it("shows every check, and a failing one with its fix command; Start is disabled (AC-1.3.2)", async () => {
    await open({ preflight: rows(false), list_campaigns: { campaigns: [] } });
    expect(await screen.findByText("bash install.sh")).toBeTruthy();
    expect(screen.getByRole("button", { name: S.home.startButton }).getAttribute("aria-disabled")).toBe("true");
  });

  it("does not start with an empty ask: names the field, calls nothing (AC-1.1.2)", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] } });
    await screen.findByText("registered");
    fill(S.home.repo, "/r");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    expect(await screen.findByText(S.home.errAsk)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "start_run")).toBe(false);
  });

  it("starts a run with the entered values and opens the Run view", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, start_run: handle });
    await screen.findByText("registered");
    fill(S.home.repo, " /r ");
    fill(S.home.ask, "fix the bug");
    fill(S.home.base, "main");
    fireEvent.click(screen.getByLabelText("2"));
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    await waitFor(() => expect(window.location.hash).toContain("#/run?"));
    const call = f.calls.find((c) => c.cmd === "start_run")!;
    expect(call.args).toMatchObject({ ask: "fix the bug", maxRounds: 2, base: "main", maxInFlight: 4, campaign: null, cwd: "/r" });
    expect(window.location.hash).toContain("slug=deliver-20261106-1500");
    expect(window.location.hash).toContain("id=20261106-1500");
  });

  it("shows the core's refusal and stays on Home", async () => {
    await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, start_run: () => { throw { error: "invalid_input", detail: "base: is not a branch or commit in this repository" }; } });
    await screen.findByText("registered");
    fill(S.home.repo, "/r"); fill(S.home.ask, "x");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    expect((await screen.findByRole("alert")).textContent).toContain("is not a branch or commit");
    expect(window.location.hash).toBe("");
  });

  it("lists campaigns with N of M passing and resumes by slug, with the goal as the ask", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [camp(), { slug: "bad", error: "unreadable" }] }, start_run: handle });
    expect(await screen.findByText("2 of 5 passing")).toBeTruthy();
    expect(screen.getByText("unreadable")).toBeTruthy();
    expect(screen.getByRole("button", { name: S.home.resume }).getAttribute("aria-disabled")).toBe("true"); // no repo yet
    fill(S.home.repo, "/r");
    fireEvent.click(screen.getByRole("button", { name: S.home.resume }));
    await waitFor(() => expect(window.location.hash).toContain("#/run?"));
    expect(f.calls.find((c) => c.cmd === "start_run")!.args).toMatchObject({ campaign: "c1", ask: "ship it", cwd: "/r" });
  });

  it("refreshes the campaign list when the core says something changed", async () => {
    let list = [camp()];
    const f = await open({ preflight: rows(true), list_campaigns: () => ({ campaigns: list }) });
    await screen.findByText(/c1/);
    list = [camp(), camp({ slug: "c2", goal: "other" })];
    act(() => f.emit("changed", ["c2"]));
    expect(await screen.findByText(/c2/)).toBeTruthy();
  });

  it("shows a message, not a blank, when the checks cannot run", async () => {
    await open({ preflight: () => { throw { error: "io_error", detail: "no" }; }, list_campaigns: { campaigns: [] } });
    expect((await screen.findByRole("alert")).textContent).toContain(S.home.preflightFailed);
  });
});
