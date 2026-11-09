// Shell B (FG-03): project tabs in the title bar, the project column, view tabs, the inspector, the status bar.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";
import { Shell } from "../src/Shell";
import { useRoute } from "../src/router";
import { REGISTRY } from "../src/screens/registry";
import { projectId } from "../src/projects/adapter";
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

describe("Preferences inside a project the core has not registered (PR 100 review)", () => {
  const view = { schema: "vajra.preferences/1", rev: 0, project_id: null, caps: { active_runs: 1, workers_per_run: 16, editable: false }, global: { ...DEFAULT_PREFERENCES }, overrides: {}, effective: { ...DEFAULT_PREFERENCES }, sources: Object.fromEntries(Object.keys(DEFAULT_PREFERENCES).map((k) => [k, "global"])) };
  it("Given a folder-derived project id, Then preferences are read as global defaults and Start is not blocked", async () => {
    // The core refuses any project scope that is not a T1 id, as the real store does.
    boot(`#/p/${API}/orch`, { preflight: { rows: [], start_enabled: true }, preferences_read: (a?: Record<string, unknown>) => a?.projectId ? Promise.reject(new Error("invalid_project")) : view });
    await waitFor(() => expect(ipc.calls.some((c) => c.cmd === "preferences_read")).toBe(true));
    expect(ipc.calls.filter((c) => c.cmd === "preferences_read").map((c) => c.args?.projectId ?? null)).toEqual(expect.arrayContaining([null]));
    expect(ipc.calls.some((c) => c.cmd === "preferences_read" && c.args?.projectId)).toBe(false);
    await waitFor(() => expect(screen.getByRole("button", { name: S.home.startButton }).getAttribute("aria-disabled")).not.toBe("true"));
  });
});
