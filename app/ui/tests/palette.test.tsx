import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Shell } from "../src/Shell";
import { projectId } from "../src/projects/adapter";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

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

const node = (id: string, kind: string, title: string, status: string, children: unknown[] = []) =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children });
const crew = node("lead", "lead", "Lead", "in_progress", [
  node("run", "run", "Run r1", "in_progress", [
    node("agent:backend", "agent", "Backend", "in_progress"),
    node("agent:frontend", "agent", "Frontend", "pending"),
  ]),
]);

const boot = (hash = "") => {
  window.location.hash = hash;
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
  const m = new Map<string, string>();
  Object.defineProperty(window, "localStorage", { configurable: true, value: { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); } } });
  setTransport(fakeIpc({
    preflight: { rows: [], start_enabled: false },
    list_campaigns: { campaigns: [
      { slug: "a1", goal: "API work", status: "done", created: "2026-11-01 09:00", updated: "2026-11-01 10:00" },
      { slug: "w1", goal: "Web work", status: "done", created: "2026-11-02 09:00", updated: "2026-11-02 10:00" },
    ] },
    resume_targets: { targets: { a1: { session_id: "s", run_id: "r", cwd: "/src/api" }, w1: { session_id: "s", run_id: "r", cwd: "/src/web" } } },
    active_runs: { count: 0, runs: [] }, inbox: { count: 0, items: [] }, crew,
  }).transport);
  render(<Shell />);
};
const cmdK = () => act(() => { fireEvent.keyDown(window, { key: "k", metaKey: true }); });
const box = () => screen.getByRole("combobox", { name: S.palette.search, hidden: true });
// The palette is mounted on first open (idle memory), so no palette at all counts as closed.
const dialogOpen = () => { const b = screen.queryByRole("combobox", { name: S.palette.search, hidden: true }); return !!b && (b.closest("dialog") as HTMLDialogElement).open; };
const options = () => screen.getAllByRole("option", { hidden: true });

describe("Command palette in the shell", () => {
  it("opens and closes with Cmd+K, and offers the other projects with their keys, the views and the panels", async () => {
    boot();
    await screen.findByRole("navigation", { name: S.frame.projects });
    expect(dialogOpen()).toBe(false);
    expect(document.querySelector("dialog")).toBeNull(); // nothing of the palette in the idle window
    cmdK();
    expect(dialogOpen()).toBe(true);
    const labels = options().map((o) => o.querySelector("span")?.textContent);
    expect(labels).toContain(S.frame.goProject("web"));
    expect(labels).not.toContain(S.frame.goProject("api")); // the project you are in
    expect(labels).toContain(S.frame.show(S.views.board));
    expect(labels).not.toContain(S.frame.show(S.views.orch)); // the view you are on
    expect(labels).toContain(S.frame.hideColumn);
    expect(labels).toContain(S.frame.openSettings);
    const web = options().find((o) => o.textContent?.startsWith(S.frame.goProject("web")))!;
    expect(web.getAttribute("aria-keyshortcuts")).toBe("Meta+2");
    cmdK();
    expect(dialogOpen()).toBe(false);
  });

  it("on a Mac, Control+K is the terminal's and never opens the palette; Cmd+K still does", async () => {
    boot();
    await screen.findByRole("navigation", { name: S.frame.projects });
    act(() => { fireEvent.keyDown(window, { key: "k", ctrlKey: true }); });
    expect(dialogOpen()).toBe(false);
    cmdK();
    expect(dialogOpen()).toBe(true);
  });

  it("opens from the title bar's search field too", async () => {
    boot();
    const btn = await screen.findByRole("button", { name: S.frame.search });
    expect(btn.getAttribute("aria-keyshortcuts")).toBe("Meta+K");
    fireEvent.click(btn);
    expect(dialogOpen()).toBe(true);
  });

  it("runs the chosen command: type, Enter, and the project changes", async () => {
    boot();
    await screen.findByRole("navigation", { name: S.frame.projects });
    cmdK();
    fireEvent.change(box(), { target: { value: "project web" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    await waitFor(() => expect(window.location.hash).toBe(`#/p/${projectId("/src/web")}/orch`));
    expect(dialogOpen()).toBe(false);
  });

  it("adds the screen's own commands: Home offers Check again", async () => {
    boot();
    await screen.findByRole("heading", { name: S.home.start });
    cmdK();
    expect(options().some((o) => o.textContent === S.home.preflightRetry)).toBe(true);
  });

  it("on Crew, goes to a worker by name and selects it in the tree; Home's commands are gone", async () => {
    boot();
    await screen.findByRole("heading", { name: S.home.start });
    act(() => { window.location.hash = "#/crew?slug=camp&id=r1"; });
    await screen.findByRole("tree", { name: S.crew.treeLabel });
    cmdK();
    await waitFor(() => expect(options().some((o) => o.textContent?.startsWith(S.palette.goWorker("Frontend")))).toBe(true));
    expect(options().some((o) => o.textContent === S.home.preflightRetry)).toBe(false);
    fireEvent.change(box(), { target: { value: "worker front" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    const row = (await screen.findAllByText("Frontend")).map((e) => e.closest("[role=treeitem]")).find(Boolean) as HTMLElement;
    await waitFor(() => expect(row.getAttribute("aria-selected")).toBe("true"));
  });
});

describe("Shell keyboard and screen-reader basics", () => {
  it("the skip link moves focus into the screen, past the sidebar", async () => {
    boot();
    await screen.findByRole("heading", { name: S.home.start });
    const skip = screen.getByRole("link", { name: S.shell.skip });
    fireEvent.click(skip);
    expect(document.activeElement?.closest("main")).toBeTruthy();
    expect(document.activeElement?.tagName).toBe("TEXTAREA"); // the ask, Home's first control
  });

  it("names the view and the project in the window title", async () => {
    boot("#/history");
    await waitFor(() => expect(document.title).toBe(S.shell.title(`${S.views.history}, api`)));
  });
});
