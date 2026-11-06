import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import {
  ActionCard, CommandPalette, CrewTree, DiffView, EvidenceRow, ResizablePanes, Table, Tabs, TerminalPane, Tooltip, parseDiff, score,
} from "../components/ui/desktop";
import type { CrewNode } from "../components/ui/desktop";

beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
});
afterEach(() => { cleanup(); vi.useRealTimers(); });
const E = "\u001b";

describe("Tabs", () => {
  function Host() {
    const [v, setV] = useState<"task" | "diff" | "log">("task");
    return <Tabs label="Worker" value={v} onChange={setV} tabs={[{ id: "task", label: "Task", panel: "task panel" }, { id: "diff", label: "Diff", panel: "diff panel" }, { id: "log", label: "Log", panel: "log panel" }]} />;
  }
  it("is a tablist with one tab stop; arrows, Home and End move focus and selection; the panel is named by its tab", () => {
    render(<Host />);
    expect(screen.getByRole("tablist", { name: "Worker" })).toBeTruthy();
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.tabIndex)).toEqual([0, -1, -1]);
    fireEvent.keyDown(tabs[0], { key: "ArrowRight" });
    expect(tabs[1].getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(tabs[1]);
    expect(screen.getByRole("tabpanel", { name: "Diff" }).textContent).toBe("diff panel");
    fireEvent.keyDown(tabs[1], { key: "End" });
    expect(tabs[2].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(tabs[2], { key: "ArrowRight" });
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(tabs[0], { key: "ArrowLeft" });
    expect(tabs[2].getAttribute("aria-selected")).toBe("true");
  });
});

describe("Table", () => {
  type R = { id: string; n: number };
  const rows: R[] = [{ id: "b", n: 2 }, { id: "a", n: 3 }, { id: "c", n: 1 }];
  const cols = [{ key: "id", header: "Criterion", sort: (x: R, y: R) => x.id.localeCompare(y.id), cell: (r: R) => r.id }, { key: "n", header: "Exit", numeric: true, cell: (r: R) => r.n }];
  it("has a caption, header cells, aria-sort and expandable rows", () => {
    render(<Table caption="Criteria" columns={cols} rows={rows} rowKey={(r) => r.id} expand={(r) => `detail ${r.id}`} />);
    expect(screen.getByRole("table", { name: "Criteria" })).toBeTruthy();
    const head = screen.getByRole("columnheader", { name: "Criterion" });
    expect(head.getAttribute("aria-sort")).toBe("none");
    fireEvent.click(within(head).getByRole("button"));
    expect(head.getAttribute("aria-sort")).toBe("ascending");
    expect(screen.getAllByRole("row").slice(1).map((r) => r.textContent)).toEqual(["Showa3", "Showb2", "Showc1"]);
    fireEvent.click(within(head).getByRole("button"));
    expect(head.getAttribute("aria-sort")).toBe("descending");
    const more = screen.getByRole("button", { name: "Details for a" });
    expect(more.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(more);
    expect(more.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("detail a")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Exit" }).hasAttribute("aria-sort")).toBe(false);
  });
});

describe("Tooltip", () => {
  it("appears after 400 ms of hover or focus, describes its child and hides on Esc", () => {
    vi.useFakeTimers();
    render(<Tooltip text="Stop this worker"><button type="button">Stop</button></Tooltip>);
    const b = screen.getByRole("button", { name: "Stop" });
    fireEvent.focus(b);
    expect(screen.queryByRole("tooltip")).toBeNull();
    act(() => { vi.advanceTimersByTime(400); });
    const tip = screen.getByRole("tooltip");
    expect(b.getAttribute("aria-describedby")).toBe(tip.id);
    fireEvent.keyDown(b, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(b.hasAttribute("aria-describedby")).toBe(false);
    fireEvent.mouseEnter(b.parentElement!);
    act(() => { vi.advanceTimersByTime(399); });
    expect(screen.queryByRole("tooltip")).toBeNull();
    fireEvent.mouseLeave(b.parentElement!);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByRole("tooltip")).toBeNull();
  });
});

describe("CommandPalette", () => {
  const mk = () => [{ id: "stop", label: "Stop worker", run: vi.fn() }, { id: "start", label: "Start run", run: vi.fn() }, { id: "open", label: "Open settings", hint: "Cmd ,", run: vi.fn() }];
  it("scores fuzzy matches in order and rejects non-matches", () => {
    expect(score("sw", "Stop worker")).toBeGreaterThan(0);
    expect(score("zz", "Stop worker")).toBe(-1);
    expect(score("start", "Start run")).toBeGreaterThan(score("sr", "Start run"));
  });
  it("is a combobox over a listbox; filters, moves with arrows, runs on Enter and closes", () => {
    const cmds = mk();
    const close = vi.fn();
    render(<CommandPalette open onClose={close} commands={cmds} />);
    const box = screen.getByRole("combobox", { name: "Search commands", hidden: true });
    expect(screen.getAllByRole("option", { hidden: true })).toHaveLength(3);
    fireEvent.change(box, { target: { value: "st" } });
    const opts = screen.getAllByRole("option", { hidden: true });
    expect(opts.map((o) => o.textContent)).toEqual(expect.arrayContaining(["Stop worker", "Start run"]));
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[0].id);
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[1].id);
    expect(opts[1].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(close).toHaveBeenCalledOnce();
    const ran = cmds.filter((c) => c.run.mock.calls.length);
    expect(ran).toHaveLength(1);
  });
  it("says so when nothing matches", () => {
    render(<CommandPalette open onClose={() => {}} commands={mk()} />);
    fireEvent.change(screen.getByRole("combobox", { hidden: true }), { target: { value: "qqq" } });
    expect(screen.getByText(/No command matches/)).toBeTruthy();
  });
});

describe("ResizablePanes", () => {
  it("exposes a separator with value, min and max; arrows, Home and End resize within the limits", () => {
    render(<ResizablePanes initial={200} min={100} max={260} label="Resize crew tree" first="tree" second="worker" />);
    const h = screen.getByRole("separator", { name: "Resize crew tree" });
    expect([h.getAttribute("aria-valuenow"), h.getAttribute("aria-valuemin"), h.getAttribute("aria-valuemax"), h.getAttribute("aria-orientation")]).toEqual(["200", "100", "260", "vertical"]);
    fireEvent.keyDown(h, { key: "ArrowRight" });
    expect(h.getAttribute("aria-valuenow")).toBe("216");
    fireEvent.keyDown(h, { key: "End" });
    fireEvent.keyDown(h, { key: "ArrowRight" });
    expect(h.getAttribute("aria-valuenow")).toBe("260");
    fireEvent.keyDown(h, { key: "Home" });
    fireEvent.keyDown(h, { key: "ArrowLeft" });
    expect(h.getAttribute("aria-valuenow")).toBe("100");
  });
  it("drags with the pointer and clamps", () => {
    render(<ResizablePanes initial={200} min={100} max={260} label="Resize" first="a" second="b" />);
    const h = screen.getByRole("separator");
    // jsdom has no PointerEvent: a MouseEvent of the pointer type carries the coordinates React reads.
    const ptr = (type: string, clientX: number) => fireEvent(h, new MouseEvent(type, { clientX, bubbles: true }));
    ptr("pointerdown", 200);
    ptr("pointermove", 230);
    expect(h.getAttribute("aria-valuenow")).toBe("230");
    ptr("pointermove", 900);
    expect(h.getAttribute("aria-valuenow")).toBe("260");
    ptr("pointerup", 900);
    ptr("pointermove", 100);
    expect(h.getAttribute("aria-valuenow")).toBe("260");
  });
});

describe("TerminalPane", () => {
  it("is a labelled, focusable, read-only log", () => {
    render(<TerminalPane lines={["hello"]} />);
    const t = screen.getByRole("log", { name: "Run terminal, read only" });
    expect(t.tabIndex).toBe(0);
    expect(t.getAttribute("aria-live")).toBe("off");
    cleanup();
    render(<TerminalPane lines={["hello"]} takenOver />);
    expect(screen.getByRole("log", { name: "Run terminal, taken over" })).toBeTruthy();
  });
  it("renders only the visible rows of a 100,000 line scrollback", () => {
    const lines = Array.from({ length: 100000 }, (_, i) => `line ${i}`);
    const { container } = render(<TerminalPane lines={lines} height={360} />);
    const rows = container.querySelectorAll(".v-vrow").length;
    expect(rows).toBeGreaterThan(0);
    expect(rows).toBeLessThan(60);
  });
  it("follows the end of a live log", () => {
    const lines = Array.from({ length: 1000 }, (_, i) => `line ${i}`);
    render(<TerminalPane lines={lines} height={360} />);
    expect(screen.getByRole("log").scrollTop).toBe(1000 * 18 - 360);
  });
  it("shows hyperlink labels and clipboard sequences as inert text: no anchors, no escapes, colors only through classes", () => {
    const { container } = render(<TerminalPane lines={[`${E}]8;;https://evil.example${E}\\open${E}]8;;${E}\\ ${E}]52;c;ZXZpbA==\u0007${E}[31mred${E}[0m ${E}[2J<b>bold</b><img src=x onerror=alert(1)>`]} />);
    const text = container.querySelector(".v-term-line")!;
    expect(container.querySelector("a")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("b")).toBeNull();
    expect(text.textContent).toBe("open red <b>bold</b><img src=x onerror=alert(1)>");
    expect(text.textContent!.includes(E)).toBe(false);
    expect(container.querySelector(".v-ansi-red")!.textContent).toBe("red");
    expect(container.innerHTML).not.toContain("style=\"color");
  });
});

describe("CrewTree", () => {
  const nodes: CrewNode[] = [
    { id: "lead", label: "Lead", status: "in_progress", children: [{ id: "be", label: "Backend", status: "in_progress", children: [{ id: "w1", label: "Backend worker 1", status: "done" }] }, { id: "fe", label: "Frontend", status: "pending" }] },
  ];
  function Host() {
    const [sel, setSel] = useState("lead");
    return <CrewTree label="Crew" nodes={nodes} selected={sel} onSelect={setSel} />;
  }
  it("is a tree with levels, expanded state, one tab stop and full keyboard navigation", () => {
    render(<Host />);
    expect(screen.getByRole("tree", { name: "Crew" })).toBeTruthy();
    let items = screen.getAllByRole("treeitem");
    expect(items).toHaveLength(1);
    expect(items[0].getAttribute("aria-expanded")).toBe("false");
    fireEvent.keyDown(items[0], { key: "ArrowRight" });
    items = screen.getAllByRole("treeitem");
    expect(items.map((i) => i.getAttribute("aria-level"))).toEqual(["1", "2", "2"]);
    expect(items[0].getAttribute("aria-expanded")).toBe("true");
    expect(items[1].getAttribute("aria-expanded")).toBe("false");
    expect(items[2].hasAttribute("aria-expanded")).toBe(false);
    fireEvent.keyDown(items[0], { key: "ArrowRight" });
    expect(document.activeElement).toBe(items[1]);
    expect(items.map((i) => i.tabIndex)).toEqual([-1, 0, -1]);
    fireEvent.keyDown(items[1], { key: "ArrowRight" });
    items = screen.getAllByRole("treeitem");
    expect(items).toHaveLength(4);
    fireEvent.keyDown(items[1], { key: "ArrowRight" });
    expect(document.activeElement).toBe(items[2]);
    fireEvent.keyDown(items[2], { key: "ArrowLeft" });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.keyDown(items[1], { key: "ArrowLeft" });
    expect(screen.getAllByRole("treeitem")).toHaveLength(3);
    fireEvent.keyDown(screen.getAllByRole("treeitem")[1], { key: "End" });
    const last = screen.getAllByRole("treeitem")[2];
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(last, { key: "Enter" });
    expect(last.getAttribute("aria-selected")).toBe("true");
    expect(within(last).getByText("Pending")).toBeTruthy();
  });
});

describe("EvidenceRow and ActionCard", () => {
  it("EvidenceRow has the criterion as its row header and a status with text", () => {
    render(<table><tbody><EvidenceRow id="ac-1" state="fail" command="npm test" exitCode={1} evidenceFile="evidence.json" commit="4c70bf04d0643792" /></tbody></table>);
    expect(screen.getByRole("rowheader", { name: "ac-1" })).toBeTruthy();
    expect(screen.getByText("Fail")).toBeTruthy();
    expect(screen.getByText("4c70bf0")).toBeTruthy();
    expect(screen.getByText("npm test")).toBeTruthy();
  });
  it("ActionCard is an article named by its heading with a live status and wired links", () => {
    const [w, a] = [vi.fn(), vi.fn()];
    render(<ActionCard kind="redirect" worker="Backend" by="Lead" text="Use the kit Button" state="delivered" time="14:22" onOpenWorker={w} onOpenAudit={a} />);
    expect(screen.getByRole("article", { name: "Redirect for Backend" })).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe("Delivered");
    fireEvent.click(screen.getByRole("button", { name: "Open worker" }));
    fireEvent.click(screen.getByRole("button", { name: "Open audit line" }));
    expect([w.mock.calls.length, a.mock.calls.length]).toEqual([1, 1]);
  });
});

describe("DiffView", () => {
  const diff = ["diff --git a/src/a.ts b/src/a.ts", "index 1..2 100644", "--- a/src/a.ts", "+++ b/src/a.ts", "@@ -1,2 +1,2 @@", " keep", "-old line", "+new line", "diff --git a/b‮.txt b/b‮.txt", "@@ -0,0 +1 @@", `+${E}]52;c;ZXZpbA==\u0007hidden`].join("\n");
  it("parses files, counts and removes control characters", () => {
    const f = parseDiff(diff);
    expect(f.map((x) => [x.path, x.added, x.removed])).toEqual([["src/a.ts", 1, 1], ["b\\u202e.txt", 1, 0]]);
    expect(f[1].lines[1].text).toBe("hidden");
  });
  it("lists files, marks added and removed lines with a glyph and a word, and switches files", () => {
    const { container } = render(<DiffView diff={diff} />);
    expect(screen.getByRole("listbox", { name: "Changed files" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Diff of src/a.ts" })).toBeTruthy();
    const lines = [...container.querySelectorAll(".v-dl")].map((l) => l.textContent);
    expect(lines).toContain("added: +new line");
    expect(lines).toContain("removed: -old line");
    fireEvent.click(screen.getByRole("option", { name: /b\\u202e\.txt/ }));
    expect(screen.getByRole("region", { name: "Diff of b\\u202e.txt" })).toBeTruthy();
  });
  it("is honest when there is nothing to show", () => {
    render(<DiffView diff="" />);
    expect(screen.getByText("No changes against the base.")).toBeTruthy();
  });
  it("virtualizes a very large file", () => {
    const big = ["diff --git a/x b/x", ...Array.from({ length: 50000 }, (_, i) => `+line ${i}`)].join("\n");
    const { container } = render(<DiffView diff={big} height={360} />);
    expect(container.querySelectorAll(".v-vrow").length).toBeLessThan(60);
  });
});
