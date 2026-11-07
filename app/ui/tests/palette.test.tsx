import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Shell } from "../src/Shell";
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
  setTransport(fakeIpc({
    preflight: { rows: [], start_enabled: false }, list_campaigns: { campaigns: [] }, resume_targets: { targets: {} },
    active_runs: { count: 0, runs: [] }, crew,
  }).transport);
  render(<Shell />);
};
const cmdK = () => act(() => { fireEvent.keyDown(window, { key: "k", metaKey: true }); });
const box = () => screen.getByRole("combobox", { name: S.palette.search, hidden: true });
// The palette is mounted on first open (idle memory), so no palette at all counts as closed.
const dialogOpen = () => { const b = screen.queryByRole("combobox", { name: S.palette.search, hidden: true }); return !!b && (b.closest("dialog") as HTMLDialogElement).open; };
const options = () => screen.getAllByRole("option", { hidden: true });

describe("Command palette in the shell", () => {
  it("opens and closes with Cmd+K, and offers every other screen with its key", () => {
    boot();
    expect(dialogOpen()).toBe(false);
    expect(document.querySelector("dialog")).toBeNull(); // nothing of the palette in the idle window
    cmdK();
    expect(dialogOpen()).toBe(true);
    const labels = options().map((o) => o.querySelector("span")?.textContent);
    expect(labels).toContain(S.palette.goScreen(S.nav.history));
    expect(labels).not.toContain(S.palette.goScreen(S.nav.home)); // the screen you are on
    const history = options().find((o) => o.textContent?.startsWith(S.palette.goScreen(S.nav.history)))!;
    expect(history.getAttribute("aria-keyshortcuts")).toBe("Meta+6");
    cmdK();
    expect(dialogOpen()).toBe(false);
  });

  it("opens from the sidebar button too; icons leave the nav names exact", () => {
    boot();
    const btn = screen.getByRole("button", { name: S.shell.search });
    expect(btn.getAttribute("aria-keyshortcuts")).toBe("Meta+K");
    fireEvent.click(btn);
    expect(dialogOpen()).toBe(true);
    expect(screen.getByRole("link", { name: S.nav.history })).toBeTruthy();
    const icons = document.querySelectorAll(".v-shell__nav svg.v-icon");
    expect(icons.length).toBe(8); // seven screens and the search button
    icons.forEach((i) => expect(i.getAttribute("aria-hidden")).toBe("true"));
  });

  it("runs the chosen command: type, Enter, and the screen changes", async () => {
    boot();
    cmdK();
    fireEvent.change(box(), { target: { value: "go to hist" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    await waitFor(() => expect(window.location.hash).toBe("#/history"));
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

  it("names the screen in the window title", async () => {
    boot("#/history");
    await waitFor(() => expect(document.title).toBe(S.shell.title(S.nav.history)));
  });
});
