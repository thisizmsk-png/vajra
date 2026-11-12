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
    expect(window.location.hash).toBe("#/review?slug=a-camp");
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
    setTransport(fakeIpc({ preflight: () => { throw { error: "io_error", detail: "x" }; }, provider_list: { schema: "vajra.providers/1", providers: [] }, setup_list: { schema: "vajra.harness_setup/1", rows: [], engine: { installed: true, command: "engine setup command" } } }).transport);
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

  it("opens the report in a pane beside the table, and closes it again", async () => {
    await open({ read_report: { slug: "a-camp", integrity: "ok", text: "body" } });
    const pane = await screen.findByRole("complementary", { name: S.history.panel });
    expect(await screen.findByRole("region", { name: S.history.reportTitle("a-camp") })).toBeTruthy();
    expect(pane.textContent).toContain("a-camp");
    fireEvent.click(screen.getByRole("button", { name: S.history.closeReport }));
    expect(screen.queryByRole("complementary", { name: S.history.panel })).toBeNull();
    expect(screen.getByRole("table")).toBeTruthy();
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

  it("refreshes a row's badge from a superseded read while the panel keeps the newer one", async () => {
    // Settled verdicts, as the core cache holds them; each read settles its own campaign's entry.
    const settled: Record<string, string> = { "a-camp": "ok", "b-camp": "ok" };
    let releaseA!: (v: unknown) => void;
    const slowA = new Promise((r) => { releaseA = r; });
    const f = fakeIpc({
      list_campaigns: () => ({ campaigns: ["a-camp", "b-camp"].map((slug) => ({ slug, goal: slug, status: "done", integrity: settled[slug] })) }),
      read_report: (a?: Record<string, unknown>) => a?.slug === "a-camp"
        ? slowA.then(() => { settled["a-camp"] = "failed"; return { slug: "a-camp", integrity: "integrity_failed", text: null }; })
        : { slug: "b-camp", integrity: "ok", text: "b body" },
    });
    setTransport(f.transport);
    render(<History />);
    const rowOf = (slug: string) => within(screen.getByRole("table")).getAllByRole("row").find((r) => within(r).queryByRole("button", { name: slug }))!;
    await screen.findByRole("table");
    fireEvent.click(screen.getByRole("button", { name: S.history.report("a-camp") }));
    fireEvent.click(screen.getByRole("button", { name: S.history.report("b-camp") }));
    expect(await screen.findByRole("region", { name: S.history.reportTitle("b-camp") })).toBeTruthy();
    const lists = () => f.calls.filter((c) => c.cmd === "list_campaigns").length;
    await waitFor(() => expect(lists()).toBe(2)); // initial load + B's refresh
    expect(within(rowOf("a-camp")).getByText(S.history.integrityOk)).toBeTruthy();
    releaseA(undefined);
    await waitFor(() => expect(within(rowOf("a-camp")).getByText(S.history.integrityFailed)).toBeTruthy());
    expect(screen.getByRole("region", { name: S.history.reportTitle("b-camp") }).textContent).toContain("b body");
    expect(screen.queryByText(S.history.reportFailed)).toBeNull();
  });

  it.each([
    ["report", S.history.report("a-camp"), "read_report", { slug: "a-camp", integrity: "ok", text: "late body" }],
    ["verify", S.history.verify("a-camp"), "verify_campaign", { slug: "a-camp", ok: true, restore: null }],
  ])("keeps the pane closed when a pending %s reply arrives after the user closed it", async (_k, button, cmd, reply) => {
    let release!: () => void;
    const slow = new Promise<void>((r) => { release = r; });
    const f = fakeIpc({ list_campaigns: camps, [cmd]: () => slow.then(() => reply) });
    setTransport(f.transport);
    render(<History />);
    await screen.findByRole("table");
    fireEvent.click(screen.getByRole("button", { name: button }));
    expect(await screen.findByRole("complementary", { name: S.history.panel })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: S.history.closeReport }));
    expect(screen.queryByRole("complementary", { name: S.history.panel })).toBeNull();
    const lists = () => f.calls.filter((c) => c.cmd === "list_campaigns").length;
    expect(lists()).toBe(1);
    release();
    await waitFor(() => expect(lists()).toBe(2)); // the shared integrity verdict still refreshes the table
    expect(screen.queryByRole("complementary", { name: S.history.panel })).toBeNull();
    expect(screen.queryByText(/late body/)).toBeNull();
  });

  it("verifies now and offers the restore command when the signature fails", async () => {
    const f = fakeIpc({ list_campaigns: list("ok"), verify_campaign: { slug: "a-camp", ok: false, restore: "vajra-campaign.sh resume a-camp --from-checkpoint 2" } });
    setTransport(f.transport);
    render(<History />);
    fireEvent.click(await screen.findByRole("button", { name: S.history.verify("a-camp") }));
    expect(await screen.findByText(/--from-checkpoint 2/)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "verify_campaign" && c.args?.slug === "a-camp")).toBe(true);
  });

  it("escapes bidi overrides, isolates, direction marks and line separators", async () => {
    await open({ read_report: { slug: "a-camp", integrity: "ok", text: "pay \u202eevil\u202c \u2066x\u2069 \u061c\u200e\u200f\u2028\u2029 done" } });
    const pre = await screen.findByRole("region", { name: S.history.reportTitle("a-camp") });
    expect(pre.textContent).toBe("pay \\u202eevil\\u202c \\u2066x\\u2069 \\u061c\\u200e\\u200f\\u2028\\u2029 done");
  });

  it.each([
    ["ok", S.history.integrityOk],
    ["no_report", S.history.integrityOk],
    ["integrity_failed", S.history.integrityFailed],
    ["unknown", S.history.integrityUnknown],
  ])("badges the report panel from the read's own %s result", async (integrity, badge) => {
    // The listing says ok throughout, so a failed or unknown badge can only come from the read itself.
    await open({ read_report: { slug: "a-camp", integrity, text: integrity === "ok" ? "body" : null } });
    const live = await waitFor(() => {
      const el = document.querySelector("[aria-live]") as HTMLElement;
      expect(el?.textContent).not.toContain(S.history.reportLoading);
      return el;
    });
    expect(within(live).getByText(badge)).toBeTruthy();
  });

  it("reloads the table after a read so the row badge matches the verdict that read settled", async () => {
    let settled = "ok";
    const f = fakeIpc({
      list_campaigns: () => list(settled),
      read_report: () => { settled = "failed"; return { slug: "a-camp", integrity: "integrity_failed", text: null }; },
    });
    setTransport(f.transport);
    render(<History />);
    const table = await screen.findByRole("table");
    expect(within(table).getByText(S.history.integrityOk)).toBeTruthy();
    fireEvent.click(within(table).getByRole("button", { name: S.history.report("a-camp") }));
    await waitFor(() => expect(within(screen.getByRole("table")).getByText(S.history.integrityFailed)).toBeTruthy());
    const read = f.calls.findIndex((c) => c.cmd === "read_report");
    expect(f.calls.slice(read).some((c) => c.cmd === "list_campaigns")).toBe(true);
  });

  it("drops a slow reply for an earlier click once a newer request is in flight", async () => {
    let release!: (v: unknown) => void;
    const slow = new Promise((r) => { release = r; });
    setTransport(fakeIpc({
      list_campaigns: list("ok"),
      read_report: () => slow,
      verify_campaign: { slug: "a-camp", ok: true, restore: null },
    }).transport);
    render(<History />);
    fireEvent.click(await screen.findByRole("button", { name: S.history.report("a-camp") }));
    fireEvent.click(screen.getByRole("button", { name: S.history.verify("a-camp") }));
    expect(await screen.findByText(S.history.verifyOk)).toBeTruthy();
    release({ slug: "a-camp", integrity: "ok", text: "stale report body" });
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByText(/stale report body/)).toBeNull();
    expect(screen.getByText(S.history.verifyOk)).toBeTruthy();
  });

  it("shows a read error as an alert", async () => {
    await open({ read_report: new Error("x") });
    expect(await screen.findByRole("alert")).toBeTruthy();
  });
});

describe("escapeControls (red team S3)", () => {
  it("makes zero-width, soft hyphen, variation selectors and tag characters visible", async () => {
    const { escapeControls } = await import("../src/screens/history/History");
    const tags = [..."deploy"].map((c) => String.fromCodePoint(0xe0000 + c.charCodeAt(0))).join("");
    const out = escapeControls(`ok​‍⁠﻿­️${tags}\u{e0100}.`);
    expect(out).toBe("ok\\u200b\\u200d\\u2060\\ufeff\\u00ad\\ufe0f\\u{e0064}\\u{e0065}\\u{e0070}\\u{e006c}\\u{e006f}\\u{e0079}\\u{e0100}.");
  });
});
