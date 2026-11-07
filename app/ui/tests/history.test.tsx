import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { History } from "../src/screens/history/History";
import { Settings } from "../src/screens/settings/Settings";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const camps = {
  campaigns: [
    { slug: "b-camp", goal: "second", status: "done", passing: 5, total: 5, updated: "2026-11-02" },
    { slug: "a-camp", goal: "first", status: "active", passing: 1, total: 4, updated: "2026-11-05" },
    { slug: "bad", error: "bad signature" },
  ],
};

describe("History", () => {
  it("lists campaigns with passing N/M and flags an unreadable one", async () => {
    setTransport(fakeIpc({ list_campaigns: camps }).transport);
    render(<History />);
    const table = await screen.findByRole("table");
    expect(within(table).getByText("1/4")).toBeTruthy();
    expect(within(table).getByText(/bad signature/)).toBeTruthy();
  });

  it("sorts by keyboard-reachable header buttons", async () => {
    setTransport(fakeIpc({ list_campaigns: camps }).transport);
    render(<History />);
    const table = await screen.findByRole("table");
    const first = () => within(table).getAllByRole("row")[1].textContent;
    expect(first()).toContain("b-camp");
    fireEvent.click(within(table).getByRole("button", { name: S.history.colSlug }));
    expect(first()).toContain("a-camp");
    expect(within(table).getByRole("columnheader", { name: S.history.colSlug }).getAttribute("aria-sort")).toBe("ascending");
  });

  it("opens a campaign in the Run view", async () => {
    setTransport(fakeIpc({ list_campaigns: camps }).transport);
    render(<History />);
    fireEvent.click(await screen.findByRole("button", { name: "a-camp" }));
    expect(window.location.hash).toBe("#/run?slug=a-camp");
  });

  it("says when there are no campaigns", async () => {
    setTransport(fakeIpc({ list_campaigns: { campaigns: [] } }).transport);
    render(<History />);
    expect(await screen.findByText(S.history.empty)).toBeTruthy();
  });

  const search = async (kb: unknown) => {
    const f = fakeIpc({ list_campaigns: { campaigns: [] }, kb_search: kb });
    setTransport(f.transport);
    render(<History />);
    await screen.findByText(S.history.empty);
    fireEvent.change(screen.getByLabelText(S.history.query), { target: { value: "x; rm -rf /" } });
    fireEvent.change(screen.getByLabelText(S.history.repo), { target: { value: "/r" } });
    fireEvent.click(screen.getByRole("button", { name: S.history.go }));
    return f;
  };

  it("passes the query as one value and lists hits (AC-6.2.1)", async () => {
    const f = await search({ hits: [{ id: "k1", kind: "decision", title: "use sqlite" }], code: 0 });
    expect(await screen.findByText("use sqlite")).toBeTruthy();
    expect(f.calls.find((c) => c.cmd === "kb_search")!.args).toEqual({ query: "x; rm -rf /", k: 20, repo: "/r" });
    expect(f.calls.every((c) => c.cmd !== "take_over")).toBe(true);
  });

  it("says 'no results' for exit 0 with no hits and 'unavailable' with the code otherwise (AC-6.2.2)", async () => {
    await search({ hits: [], code: 0 });
    expect(await screen.findByText(S.history.none)).toBeTruthy();
    cleanup();
    await search({ hits: [], code: 2 });
    expect(await screen.findByText(S.history.unavailable(2))).toBeTruthy();
  });

  it("does not search with an empty query", async () => {
    const f = fakeIpc({ list_campaigns: { campaigns: [] }, kb_search: { hits: [], code: 0 } });
    setTransport(f.transport);
    render(<History />);
    await screen.findByText(S.history.empty);
    fireEvent.click(screen.getByRole("button", { name: S.history.go }));
    expect(await screen.findByText(S.history.errQuery)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "kb_search")).toBe(false);
  });
});

describe("Settings", () => {
  it("shows the checks with the fix for a failing one, and a message when they cannot run", async () => {
    setTransport(fakeIpc({ preflight: { rows: [{ name: "jq", ok: false, detail: "missing", fix: "brew install jq" }], start_enabled: false } }).transport);
    render(<Settings />);
    expect(await screen.findByText("brew install jq")).toBeTruthy();
    cleanup();
    setTransport(fakeIpc({ preflight: () => { throw { error: "io_error", detail: "x" }; } }).transport);
    render(<Settings />);
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain(S.settings.failed));
  });
});

describe("History report and integrity (S6)", () => {
  const list = (integrity: string) => ({ campaigns: [{ slug: "a-camp", goal: "first", status: "done", passing: 4, total: 4, updated: "2026-11-05", integrity }] });
  const open = async (replies: Record<string, unknown>) => {
    const f = fakeIpc({ list_campaigns: list("ok"), ...replies });
    setTransport(f.transport);
    render(<History />);
    fireEvent.click(await screen.findByRole("button", { name: S.history.report("a-camp") }));
    return f;
  };

  it("shows each campaign's integrity badge, and a failed one says so", async () => {
    setTransport(fakeIpc({ list_campaigns: list("failed") }).transport);
    render(<History />);
    expect(await screen.findByText(S.history.integrityFailed)).toBeTruthy();
  });

  it("renders the stored report as text with control characters escaped", async () => {
    const f = await open({ read_report: { slug: "a-camp", integrity: "ok", text: "Branch b\u0000\u001b[31mred\nline 2\ttab" } });
    const pre = await screen.findByRole("region", { name: S.history.reportTitle("a-camp") });
    expect(pre.textContent).toContain("Branch b\\u0000\\u001b[31mred");
    expect(pre.textContent).toContain("line 2\ttab");
    expect(f.calls.find((c) => c.cmd === "read_report")?.args).toEqual({ slug: "a-camp" });
  });

  it.each([
    ["no_report", S.history.noReport],
    ["integrity_failed", S.history.reportFailed],
    ["not_text", S.history.notText],
    ["unknown", S.history.reportUnknown],
  ])("says %s and shows no text", async (integrity, message) => {
    await open({ read_report: { slug: "a-camp", integrity, text: "should never be shown" } });
    expect(await screen.findByText(message)).toBeTruthy();
    expect(screen.queryByText(/should never be shown/)).toBeNull();
  });

  it("verifies now and offers the restore command when the signature fails", async () => {
    const f = fakeIpc({ list_campaigns: list("ok"), verify_campaign: { slug: "a-camp", ok: false, restore: "vajra-campaign.sh resume a-camp --from-checkpoint 2" } });
    setTransport(f.transport);
    render(<History />);
    fireEvent.click(await screen.findByRole("button", { name: S.history.verify("a-camp") }));
    expect(await screen.findByText(/--from-checkpoint 2/)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "verify_campaign" && c.args?.slug === "a-camp")).toBe(true);
  });

  it("shows a read error as an alert", async () => {
    await open({ read_report: new Error("x") });
    expect(await screen.findByRole("alert")).toBeTruthy();
  });
});
