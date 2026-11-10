// Shell B (FG-03): project tabs in the title bar, the project column, view tabs, the inspector, the status bar.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";
import { Shell } from "../src/Shell";
import { useRoute } from "../src/router";
import { REGISTRY } from "../src/screens/registry";
import { projectId } from "../src/projects/adapter";
import { inboxBadge } from "../src/shell/ViewTabs";
import { S } from "../strings";
import { DEFAULT_PREFERENCES } from "../src/preferenceContract";

const node = (id: string, kind: string, title: string, status: string, children: unknown[] = [], role: string | null = null) =>
  ({ id, kind, title, engine_id: null, role, status, overlays: [], children });
const crew = node("lead", "lead", "Lead", "in_progress", [
  node("run:r1", "run", "Add rate limits", "in_progress", [
    node("task:t2", "task", "t2 Token bucket", "in_progress", [node("agent:backend", "worker", "Backend", "in_progress", [], "Backend")]),
    node("task:t4", "task", "t4 Retry-After", "question", [node("agent:sepoy-1", "worker", "Backend worker 1", "question", [], "Backend worker 1")]),
  ]),
]);
const API = projectId("/src/api");
const WEB = projectId("/src/web");
const replies = (over: Record<string, unknown> = {}) => ({
  preflight: { rows: [], start_enabled: false },
  list_campaigns: { campaigns: [
    { slug: "api-1", goal: "Add rate limits", status: "active", passing: 1, total: 5, created: "2026-11-01 09:00", updated: "2026-11-08 09:00" },
    { slug: "web-1", goal: "Fix the login loop", status: "stopped", passing: 2, total: 4, created: "2026-11-03 09:00", updated: "2026-11-07 09:00" },
    { slug: "loose", goal: "Started in a terminal", status: "done", updated: "2026-11-02 09:00" },
  ] },
  resume_targets: { targets: { "api-1": { session_id: "s", run_id: "r1", cwd: "/src/api" }, "web-1": { session_id: "s2", run_id: "r2", cwd: "/src/web" } } },
  active_runs: { count: 1, runs: [{ run_id: "r1", campaign: "api-1", stream: "s", session_id: "s" }] },
  inbox: { count: 2, items: [
    { id: "i1", kind: "question", campaign: "api-1", origin: "app", created: "", age_s: 5, title: "Which key?", body: "", actions: [] },
    { id: "i2", kind: "plan", campaign: "api-1", origin: "app", created: "", age_s: 9, title: "Deploy", body: "", actions: [] },
  ] },
  crew,
  ...over,
});

let ipc: ReturnType<typeof fakeIpc>;
const boot = (hash = "", over: Record<string, unknown> = {}) => {
  window.location.hash = hash;
  ipc = fakeIpc(replies(over));
  setTransport(ipc.transport);
  return render(<Shell />);
};
const tabs = () => within(screen.getByRole("navigation", { name: S.frame.projects }));
const key = (k: string, mods: Partial<KeyboardEventInit> = { metaKey: true }) => act(() => { fireEvent.keyDown(window, { key: k, ...mods }); });

/** An in-memory Storage: the test runtime's own localStorage is not a usable one. */
function memoryStorage(): Storage {
  const m = new Map<string, string>();
  return {
    get length() { return m.size; }, clear: () => m.clear(), getItem: (k) => m.get(k) ?? null, key: (i) => [...m.keys()][i] ?? null,
    removeItem: (k) => { m.delete(k); }, setItem: (k, v) => { m.set(k, String(v)); },
  };
}
beforeEach(() => {
  Object.defineProperty(window, "localStorage", { configurable: true, value: memoryStorage() });
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
});
afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; });

describe("Shell B frame (FG-03)", () => {
  it("has no static list of screens: the title bar lists projects, Unsorted last", async () => {
    boot();
    await waitFor(() => expect(tabs().getAllByRole("link")).toHaveLength(3));
    expect(tabs().getAllByRole("link").map((l) => l.textContent)).toEqual([
      expect.stringContaining("api"), expect.stringContaining("web"), expect.stringContaining(S.projects.unsorted),
    ]);
    for (const old of [S.nav.home, S.nav.run, S.nav.crew, S.nav.chat, S.nav.history]) {
      expect(screen.queryByRole("link", { name: old })).toBeNull();
    }
  });

  it("keeps urgent states visible on the tabs: needs-you count, stopped", async () => {
    boot();
    const api = await waitFor(() => tabs().getByRole("link", { name: S.frame.tabName("api", S.frame.needsYou(2)) }));
    expect(api.textContent).toContain(S.frame.needsYou(2));
    expect(tabs().getByRole("link", { name: S.frame.tabName("web", S.frame.stopped) })).toBeTruthy();
  });

  it("opens the project that owns the live run, with its Director-General leading that run and its workers", async () => {
    boot();
    const col = await screen.findByRole("complementary", { name: S.column.label("api") });
    expect(tabs().getByRole("link", { name: /^api/ }).getAttribute("aria-current")).toBe("page");
    await waitFor(() => expect(within(col).getByText(S.column.leading("r1"))).toBeTruthy());
    expect(within(col).getByText(S.views.orch)).toBeTruthy();
    expect(within(col).getByText("Backend")).toBeTruthy();
    expect(within(col).getByText("t4 Retry-After")).toBeTruthy();
  });

  it("switching project shows only that project's sessions", async () => {
    boot();
    const col = await screen.findByRole("complementary", { name: S.column.label("api") });
    await waitFor(() => expect(within(col).getAllByText("Add rate limits").length).toBeGreaterThan(0));
    expect(within(col).queryAllByText("Fix the login loop")).toHaveLength(0);
    fireEvent.click(tabs().getByRole("link", { name: /^web/ }));
    const web = await screen.findByRole("complementary", { name: S.column.label("web") });
    expect(within(web).getAllByText("Fix the login loop").length).toBeGreaterThan(0);
    expect(within(web).queryAllByText("Add rate limits")).toHaveLength(0);
    expect(within(web).getByText(S.column.noRun)).toBeTruthy();
  });

  it("Cmd+1..n switch projects in tab order", async () => {
    boot();
    await waitFor(() => expect(tabs().getAllByRole("link")).toHaveLength(3));
    key("2");
    expect(window.location.hash.startsWith(`#/p/${WEB}/`)).toBe(true);
    key("1");
    expect(window.location.hash.startsWith(`#/p/${API}/`)).toBe(true);
  });

  it("Cmd+B folds the project column away and back, and remembers it", async () => {
    const view = boot();
    await screen.findByRole("complementary", { name: S.column.label("api") });
    key("b");
    expect(screen.queryByRole("complementary", { name: S.column.label("api") })).toBeNull();
    view.unmount();
    boot();
    await waitFor(() => expect(tabs().getAllByRole("link")).toHaveLength(3));
    expect(screen.queryByRole("complementary", { name: S.column.label("api") })).toBeNull();
    key("b");
    expect(await screen.findByRole("complementary", { name: S.column.label("api") })).toBeTruthy();
  });

  it("starts with the inspector closed below 1440 px wide, and Cmd+I opens it", async () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1280 });
    boot();
    await screen.findByRole("complementary", { name: S.column.label("api") });
    expect(screen.queryByRole("complementary", { name: S.frame.inspector })).toBeNull();
    key("i");
    expect(screen.getByRole("complementary", { name: S.frame.inspector })).toBeTruthy();
  });

  it("the column and inspector toggles keep one name; aria-pressed carries the state (WCAG 4.1.2)", async () => {
    boot();
    await screen.findByRole("complementary", { name: S.column.label("api") });
    const column = screen.getByRole("button", { name: S.frame.column });
    const inspector = screen.getByRole("button", { name: S.frame.inspector });
    expect(column.getAttribute("aria-pressed")).toBe("true");
    expect(inspector.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(column);
    fireEvent.click(inspector);
    expect(screen.getByRole("button", { name: S.frame.column }).getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByRole("button", { name: S.frame.inspector }).getAttribute("aria-pressed")).toBe("false");
  });

  it("opens the inspector by default from 1440 px", async () => {
    boot();
    expect(await screen.findByRole("complementary", { name: S.frame.inspector })).toBeTruthy();
  });

  it("switches views with the header tabs, and a worker opens as its own tab", async () => {
    boot();
    const views = within(await screen.findByRole("navigation", { name: S.frame.views }));
    expect(views.getByRole("link", { name: S.views.orch }).getAttribute("aria-current")).toBe("page");
    const col = await screen.findByRole("complementary", { name: S.column.label("api") });
    fireEvent.click(await within(col).findByRole("button", { name: /Backend worker 1/ }));
    await waitFor(() => expect(window.location.hash).toBe(`#/p/${API}/worker?slug=api-1&id=r1&worker=sepoy-1`));
    expect((await views.findByRole("link", { name: "Backend worker 1" })).getAttribute("aria-current")).toBe("page");
    fireEvent.click(views.getByRole("button", { name: S.frame.closeTab("Backend worker 1") }));
    expect(views.queryByRole("link", { name: "Backend worker 1" })).toBeNull();
    expect(window.location.hash).toBe(`#/p/${API}/orch`);
  });

  it("Open on an inbox item goes to the project the core filed it under, even when its campaign's folder says another", async () => {
    const A = "p_01JC3H0000000000000000000A";
    const W = "p_01JC3H0000000000000000000B";
    const stored = (id: string, root: string) => ({ schema: "vajra.project/1", id, name: root.split("/").pop(), root, root_key: {}, created: "2026-11-01T09:00:00Z", archived: null, rev: 1 });
    boot("#/inbox", {
      project_list: { projects: [stored(A, "/src/api"), stored(W, "/src/web")], skipped: 0, truncated: false },
      inbox: { count: 1, items: [{ id: "i9", kind: "question", campaign: "api-1", project_id: W, origin: "app", created: "", age_s: 5, title: "Which key?", body: "", actions: [] }] },
    });
    const open = await screen.findByRole("button", { name: S.inbox.open });
    fireEvent.click(open);
    await waitFor(() => expect(window.location.hash).toContain(`#/p/${W}/`));
    await waitFor(() => expect(tabs().getByRole("link", { name: /^web/ }).getAttribute("aria-current")).toBe("page"));
    expect(tabs().getByRole("link", { name: /^api/ }).getAttribute("aria-current")).not.toBe("page");
  });

  it("History opens a campaign of another project in that project's frame", async () => {
    boot(`#/p/${API}/history`);
    fireEvent.click(await screen.findByRole("button", { name: "web-1" }));
    await waitFor(() => expect(tabs().getByRole("link", { name: /^web/ }).getAttribute("aria-current")).toBe("page"));
    expect(tabs().getByRole("link", { name: /^api/ }).getAttribute("aria-current")).not.toBe("page");
  });

  it("places an old campaign link in its project: #/run?slug=web-1 is web's Review", async () => {
    boot("#/run?slug=web-1");
    await waitFor(() => expect(tabs().getByRole("link", { name: /^web/ }).getAttribute("aria-current")).toBe("page"));
    expect(within(screen.getByRole("navigation", { name: S.frame.views })).getByRole("link", { name: S.views.review }).getAttribute("aria-current")).toBe("page");
  });

  it("has a status bar with the live runs, the phone and Settings; the phone goes to Settings > Mobile", async () => {
    boot();
    const bar = screen.getByRole("contentinfo", { name: S.frame.status });
    await waitFor(() => expect(within(bar).getByText(S.frame.runs(1))).toBeTruthy());
    fireEvent.click(within(bar).getByRole("button", { name: S.frame.phone }));
    expect(window.location.hash).toContain("settings?section=mobile");
    fireEvent.click(within(bar).getByRole("button", { name: S.views.settings }));
    expect(window.location.hash).toMatch(/\/settings$/);
  });

  it("with no projects yet: no column, the launcher, and Add project", async () => {
    boot("", { list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, active_runs: { count: 0, runs: [] }, inbox: { count: 0, items: [] } });
    expect(await screen.findByRole("heading", { name: S.home.start })).toBeTruthy();
    expect(screen.getByRole("button", { name: S.frame.addProject })).toBeTruthy();
    expect(screen.queryByRole("complementary", { name: /project$/ })).toBeNull();
  });

  it("does not poll while nothing runs (idle cost)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      boot("", { active_runs: { count: 0, runs: [] } });
      await waitFor(() => expect(tabs().getAllByRole("link")).toHaveLength(3));
      const before = ipc.calls.filter((c) => c.cmd === "list_campaigns").length;
      await act(async () => { vi.advanceTimersByTime(10_000); });
      expect(ipc.calls.filter((c) => c.cmd === "list_campaigns").length).toBe(before);
    } finally { vi.useRealTimers(); }
  });

  it("contains a throwing view and recovers when you leave it", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const orig = REGISTRY.crew;
    REGISTRY.crew = () => { throw new Error("boom"); };
    boot("#/crew");
    expect(screen.getByRole("alert").textContent).toContain(S.shell.errorTitle);
    act(() => {
      window.location.hash = "#/history";
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    });
    await waitFor(() => expect(screen.queryByText(S.shell.errorTitle)).toBeNull());
    REGISTRY.crew = orig;
    err.mockRestore();
  });

  it("resets a failed view when only the params change", () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const orig = REGISTRY.review;
    REGISTRY.review = function Review() {
      const [r] = useRoute();
      if (r.params.slug === "a") throw new Error("boom");
      return <p>review {r.params.slug}</p>;
    };
    boot("#/review?slug=a");
    expect(screen.getByRole("alert")).toBeTruthy();
    act(() => {
      window.location.hash = "#/review?slug=b";
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("review b")).toBeTruthy();
    REGISTRY.review = orig;
    err.mockRestore();
  });
});

describe("The Inbox tab counts campaign items only: run proposals live on the Director-General surface", () => {
  const item = (id: string, kind: string) => ({ id, kind, campaign: kind === "run_proposal" ? undefined : "api-1", origin: "app", created: "", age_s: 1, title: id, body: "", actions: [] }) as never;
  const proposal = { id: "proposal:1", kind: "run_proposal", origin: "app", source: "from the Director-General", created: "", age_s: 1, title: "Start a run?", body: "", ask: "do it", base: "main", rounds: 2, project: "/src/api", generation: 1, proposal_sha256: "ab".repeat(32), actions: ["start", "dismiss"] };
  const inboxTab = () => screen.getByRole("link", { name: new RegExp(`^${S.views.inbox}`) });

  it("counts a full list exactly, leaving proposals out; a cut list always says '+'; a full list with nothing to count shows nothing", () => {
    expect(inboxBadge([item("p", "plan"), item("q", "question"), item("r", "run_proposal")], 3)).toBe("2");
    expect(inboxBadge([item("p", "plan"), item("r", "run_proposal")], 40)).toBe("1+");
    expect(inboxBadge([item("r", "run_proposal")], 1)).toBe("");
    // A cut list whose sent items are all proposals may still hide campaign items: it says so, never nothing.
    expect(inboxBadge([item("r", "run_proposal")], 9)).toBe("+");
    expect(inboxBadge([], 5)).toBe("+");
    expect(inboxBadge([], 0)).toBe("");
  });

  it("uses the core's campaign-only count exactly when it sends one, past the cut and with proposals left out", () => {
    expect(inboxBadge([item("p", "plan"), item("r", "run_proposal")], 120, 57)).toBe("57");
    expect(inboxBadge([item("r", "run_proposal")], 9, 0)).toBe("");
    expect(inboxBadge([item("p", "plan"), item("q", "question"), item("r", "run_proposal")], 3, 2)).toBe("2");
  });

  it("Given the core sends campaign_count with a cut list, Then the tab gives that exact number, not a '+'", async () => {
    boot(`#/p/${API}/orch`, { inbox: { count: 120, campaign_count: 57, items: [...(replies().inbox as { items: unknown[] }).items, proposal] } });
    await waitFor(() => expect(inboxTab().textContent).toBe(`${S.views.inbox} 57`));
  });

  it("Given the fixture's plan, question and proposal, Then the tab reads Inbox 2, while the project still counts all three as needing you", async () => {
    boot(`#/p/${API}/orch`, { inbox: { count: 3, items: [...(replies().inbox as { items: unknown[] }).items, proposal] } });
    await waitFor(() => expect(inboxTab().textContent).toBe(`${S.views.inbox} 2`));
    expect(tabs().getByText(S.frame.needsYou(3))).toBeTruthy();
  });

  it("Given only a run proposal waiting, Then the Inbox tab shows no count", async () => {
    boot(`#/p/${API}/orch`, { inbox: { count: 1, items: [proposal] } });
    await waitFor(() => expect(ipc.calls.some((c) => c.cmd === "inbox")).toBe(true));
    await waitFor(() => expect(inboxTab().textContent).toBe(S.views.inbox));
  });

  it("Given more waiting than the core sends, Then the tab gives the campaign count it has with a '+', not a wrong exact number", async () => {
    boot(`#/p/${API}/orch`, { inbox: { count: 120, items: [...(replies().inbox as { items: unknown[] }).items, proposal] } });
    await waitFor(() => expect(inboxTab().textContent).toBe(`${S.views.inbox} 2+`));
  });
});

describe("Preferences inside a project the core has not registered (PR 100 review)", () => {
  const view = { schema: "vajra.preferences/1", rev: 0, project_id: null, caps: { active_runs: 1, workers_per_run: 16, editable: false }, global: { ...DEFAULT_PREFERENCES }, overrides: {}, effective: { ...DEFAULT_PREFERENCES }, sources: Object.fromEntries(Object.keys(DEFAULT_PREFERENCES).map((k) => [k, "global"])) };
  it("Given a folder-derived project id, Then preferences are read as global defaults and Start is not blocked", async () => {
    // The core refuses any project scope that is not a T1 id, as the real store does.
    boot(`#/p/${API}/launch`, { preflight: { rows: [], start_enabled: true }, preferences_read: (a?: Record<string, unknown>) => a?.projectId ? Promise.reject(new Error("invalid_project")) : view });
    await waitFor(() => expect(ipc.calls.some((c) => c.cmd === "preferences_read")).toBe(true));
    expect(ipc.calls.filter((c) => c.cmd === "preferences_read").map((c) => c.args?.projectId ?? null)).toEqual(expect.arrayContaining([null]));
    expect(ipc.calls.some((c) => c.cmd === "preferences_read" && c.args?.projectId)).toBe(false);
    await waitFor(() => expect(screen.getByRole("button", { name: S.home.startButton }).getAttribute("aria-disabled")).not.toBe("true"));
  });
});

describe("Run proposals of a registered project come from its own projection (#90 review)", () => {
  const T = "p_01JC3H0000000000000000000A";
  const registered = { project_list: { projects: [{ schema: "vajra.project/1", id: T, name: "Harbor API", root: "/src/api", root_key: {}, created: "2026-11-01T09:00:00Z", archived: null, rev: 2 }], skipped: 0, truncated: false } };
  const proposal = { id: "proposal:new", kind: "run_proposal", origin: "app", source: "from the Director-General", created: "", age_s: 1, title: "Start a run?", body: "Ship the new limits", ask: "Ship the new limits", base: "main", rounds: 1, project_id: T, project: "/src/api", generation: 1, proposal_sha256: "ab".repeat(32), actions: ["start", "dismiss"] };
  // The global read is full of older items and its cut left the new proposal out.
  const full = { count: 251, campaign_count: 250, items: Array.from({ length: 200 }, (_, i) => ({ id: `task:fill-${i}.t1`, kind: "task", campaign: "api-1", project_id: T, origin: "app", created: "", age_s: 900, title: `filler ${i}`, body: "", actions: ["open"] })) };
  const state = (proposals: unknown) => ({ schema: "vajra.project_state/1", project_id: T, rev: 7, unchanged: false, active_runs: 0, generations: { project: 1 }, workers: [], receipts: [], inbox: { ok: true, count: 251, items: [] }, proposals });

  it("Given the inbox cut left a new proposal out, Then the Director-General still shows it", async () => {
    boot(`#/p/${T}/orch`, { ...registered, inbox: full, project_state: state({ ok: true, count: 1, more: 0, items: [proposal] }) });
    const region = await screen.findByRole("region", { name: S.dg.proposals });
    expect(within(region).getByText("Ship the new limits")).toBeTruthy();
    expect(ipc.calls.some((c) => c.cmd === "project_state" && c.args?.projectId === T)).toBe(true);
  });

  it("Given the projection's proposals could not be read, Then the inbox is the fallback, never an empty list", async () => {
    boot(`#/p/${T}/orch`, { ...registered, inbox: { count: 1, items: [proposal] }, project_state: state({ ok: false, error: "io_error" }) });
    const region = await screen.findByRole("region", { name: S.dg.proposals });
    expect(within(region).getByText("Ship the new limits")).toBeTruthy();
  });
});

describe("The usage chip comes from the project state (#120 review)", () => {
  const T = "p_01JC3H0000000000000000000A";
  const registered = { project_list: { projects: [{ schema: "vajra.project/1", id: T, name: "Harbor API", root: "/src/api", root_key: {}, created: "2026-11-01T09:00:00Z", archived: null, rev: 2 }], skipped: 0, truncated: false } };
  const state = (usage: unknown) => ({ schema: "vajra.project_state/1", project_id: T, rev: 7, unchanged: false, active_runs: 0, generations: { project: 1 }, workers: [], receipts: [], inbox: { ok: true, count: 0, items: [] }, proposals: { ok: true, count: 0, more: 0, items: [] }, usage });
  const usage = { schema: "vajra.provider_usage/1", usage: [{ account: "claude", read_at: "2026-11-09T12:00:00Z", source: "cli", windows: [{ kind: "weekly", used_pct: 84, resets_at: null, warn: true }], amounts: null }] };

  it("Given the project state reports a warned window, Then the status bar shows the chip", async () => {
    boot(`#/p/${T}/orch`, { ...registered, project_state: state(usage) });
    const bar = await screen.findByLabelText(S.frame.status);
    await waitFor(() => expect(within(bar).getByRole("status").textContent).toBe("Claude 84% used this week."));
  });

  it("Given the usage changes later, Then the chip follows when the core announces a newer revision", async () => {
    const at = (pct: number) => ({ ...usage, usage: [{ ...usage.usage[0], windows: [{ kind: "weekly", used_pct: pct, resets_at: null, warn: true }] }] });
    let answer: unknown = state(at(84));
    boot(`#/p/${T}/orch`, { ...registered, project_state: () => answer });
    const bar = await screen.findByLabelText(S.frame.status);
    await waitFor(() => expect(within(bar).getByRole("status").textContent).toBe("Claude 84% used this week."));
    answer = { ...state(at(91)), rev: 8 };
    act(() => ipc.emit("changed", { streams: [], snapshot_needed: false, projects: [{ project_id: T, rev: 8 }] }));
    await waitFor(() => expect(within(bar).getByRole("status").textContent).toBe("Claude 91% used this week."), { timeout: 3000 });
  });

  it("Given nothing is near its limit, Then there is no chip", async () => {
    boot(`#/p/${T}/orch`, { ...registered, project_state: state({ ...usage, usage: [{ ...usage.usage[0], windows: [{ kind: "weekly", used_pct: 10, resets_at: null, warn: false }] }] }) });
    await waitFor(() => expect(ipc.calls.some((c) => c.cmd === "project_state")).toBe(true));
    expect(within(screen.getByLabelText(S.frame.status)).queryByRole("status")).toBeNull();
  });
});

