import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import {
  AlertDialog, Badge, Button, Card, Input, ListBox, ListRow, SegmentedControl, Section, Separator, Sheet, Skeleton, StatusDot, Switch, Text, ToastRegion,
  useToasts,
} from "../components/ui/base";
import { S } from "../strings";

// jsdom has no modal dialog: model showModal/close/cancel the way browsers do.
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
});
afterEach(() => { cleanup(); vi.useRealTimers(); });

describe("Text, Badge, Card, Section, Separator", () => {
  it("Text renders the semantic element asked for", () => {
    render(<Text as="h1" variant="headline-lg">Runs</Text>);
    expect(screen.getByRole("heading", { level: 1, name: "Runs" })).toBeTruthy();
  });
  it("Badge always carries text", () => {
    render(<Badge status="warning">Needs approval</Badge>);
    expect(screen.getByText("Needs approval")).toBeTruthy();
  });
  it("an interactive Card is a button", () => {
    const f = vi.fn();
    render(<Card interactive onClick={f}>Open run</Card>);
    fireEvent.click(screen.getByRole("button", { name: "Open run" }));
    expect(f).toHaveBeenCalledOnce();
  });
  it("Section is a labelled region", () => {
    render(<Section title="Criteria">x</Section>);
    expect(screen.getByRole("region", { name: "Criteria" })).toBeTruthy();
  });
  it("Separator has its role and orientation", () => {
    render(<Separator orientation="vertical" />);
    expect(screen.getByRole("separator").getAttribute("aria-orientation")).toBe("vertical");
  });
});

describe("Button", () => {
  it("disabled uses aria-disabled, stays focusable and ignores clicks", () => {
    const f = vi.fn();
    render(<Button disabled onClick={f}>Stop worker</Button>);
    const b = screen.getByRole("button", { name: "Stop worker" });
    expect(b.getAttribute("aria-disabled")).toBe("true");
    expect((b as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(b);
    expect(f).not.toHaveBeenCalled();
  });
  it("busy is announced and ignores clicks", () => {
    const f = vi.fn();
    render(<Button busy onClick={f}>Stopping</Button>);
    const b = screen.getByRole("button", { name: "Stopping" });
    expect(b.getAttribute("aria-busy")).toBe("true");
    fireEvent.click(b);
    expect(f).not.toHaveBeenCalled();
  });
  it("an icon-only button is named by aria-label", () => {
    render(<Button aria-label="Close panel" />);
    expect(screen.getByRole("button", { name: "Close panel" })).toBeTruthy();
  });
});

describe("Input", () => {
  it("ties label, hint and error to the control", () => {
    render(<Input label="Branch" hint="A branch or commit" error="Not found" value="x" onChange={() => {}} />);
    const c = screen.getByLabelText("Branch");
    expect(c.getAttribute("aria-invalid")).toBe("true");
    const described = c.getAttribute("aria-describedby")!.split(" ").map((id) => document.getElementById(id)!.textContent);
    expect(described).toEqual(["A branch or commit", "Not found"]);
  });
  it("number is a native number input with the same label and error wiring", () => {
    const f = vi.fn();
    render(<Input label="Max in flight" type="number" error="1 to 16" value="4" onChange={f} />);
    const c = screen.getByRole("spinbutton", { name: "Max in flight" });
    expect(c.getAttribute("aria-invalid")).toBe("true");
    fireEvent.change(c, { target: { value: "8" } });
    expect(f).toHaveBeenCalledWith("8");
  });
  it("covers text, textarea and select", () => {
    const f = vi.fn();
    render(<><Input label="A" value="" onChange={f} /><Input label="C" type="textarea" value="" onChange={f} />
      <Input label="D" type="select" value="x" options={[{ value: "x", label: "X" }]} onChange={f} /></>);
    expect(screen.getByRole("textbox", { name: "A" })).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "C" }).tagName).toBe("TEXTAREA");
    expect(screen.getByRole("combobox", { name: "D" })).toBeTruthy();
    fireEvent.change(screen.getByRole("textbox", { name: "A" }), { target: { value: "q" } });
    expect(f).toHaveBeenCalledWith("q");
  });
});

describe("ListRow, SegmentedControl, Switch", () => {
  it("ListBox is one tab stop; arrows, Home and End move the active option; Enter and Space select it", () => {
    const [a, b] = [vi.fn(), vi.fn()];
    render(<ListBox label="Runs"><ListRow selected onSelect={a} meta="3 of 7">Run one</ListRow><ListRow onSelect={b}>Run two</ListRow><ListRow>Run three</ListRow></ListBox>);
    const box = screen.getByRole("listbox", { name: "Runs" });
    const opts = screen.getAllByRole("option");
    expect(box.tabIndex).toBe(0);
    expect(opts.every((o) => !o.hasAttribute("tabindex"))).toBe(true);
    expect(opts[0].getAttribute("aria-selected")).toBe("true");
    expect(opts[1].hasAttribute("aria-selected")).toBe(false);
    fireEvent.focus(box);
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[0].id);       // starts on the selected option
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[1].id);
    expect(opts[1].hasAttribute("data-active")).toBe(true);
    expect(opts[0].hasAttribute("data-active")).toBe(false);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(b).toHaveBeenCalledOnce();
    fireEvent.keyDown(box, { key: "End" });
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[2].id);
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[2].id);       // clamps
    fireEvent.keyDown(box, { key: "Home" });
    fireEvent.keyDown(box, { key: " " });
    expect(a).toHaveBeenCalledOnce();
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[0].id);
  });
  it("a click moves the active option, so Enter acts on the clicked row (not the previously active one)", () => {
    const [a, b] = [vi.fn(), vi.fn()];
    render(<ListBox label="Runs"><ListRow selected onSelect={a}>Run one</ListRow><ListRow onSelect={b}>Run two</ListRow></ListBox>);
    const box = screen.getByRole("listbox", { name: "Runs" });
    const opts = screen.getAllByRole("option");
    fireEvent.focus(box);
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[0].id);
    fireEvent.click(opts[1]);
    expect(b).toHaveBeenCalledOnce();
    expect(box.getAttribute("aria-activedescendant")).toBe(opts[1].id);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(b).toHaveBeenCalledTimes(2);
    expect(a).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(box);
  });
  it("lands on the first option when the list was empty at focus, and recovers when the active row is removed", () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    function Host({ rows }: { rows: string[] }) {
      return <ListBox label="Runs">{rows.map((r) => <ListRow key={r}>{r}</ListRow>)}</ListBox>;
    }
    const { rerender } = render(<Host rows={[]} />);
    const box = screen.getByRole("listbox", { name: "Runs" });
    fireEvent.focus(box);
    expect(box.hasAttribute("aria-activedescendant")).toBe(false);
    rerender(<Host rows={["one", "two", "three"]} />);
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(screen.getAllByRole("option")[0].id).toBe(box.getAttribute("aria-activedescendant")); // not row 1
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(screen.getAllByRole("option")[1].id).toBe(box.getAttribute("aria-activedescendant"));
    rerender(<Host rows={["two-new", "three"]} />);                     // the active row was removed
    expect(screen.getAllByRole("option")[0].id).toBe(box.getAttribute("aria-activedescendant"));
    expect(scroll).toHaveBeenCalled();
  });
  it("scrolls the initial option into view on focus", () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    render(<ListBox label="Runs"><ListRow>a</ListRow><ListRow selected>b</ListRow></ListBox>);
    fireEvent.focus(screen.getByRole("listbox"));
    expect(scroll).toHaveBeenCalledOnce();
    expect(screen.getByRole("listbox").getAttribute("aria-activedescendant")).toBe(screen.getAllByRole("option")[1].id);
  });
  it("ListRow can be a grid row with cells and no aria-selected", () => {
    render(<div role="grid" aria-label="Criteria"><ListRow role="row" meta="exit 0">ac-1</ListRow></div>);
    const row = screen.getByRole("row");
    expect(row.hasAttribute("aria-selected")).toBe(false);
    expect(screen.getAllByRole("gridcell").map((c) => c.textContent)).toEqual(["ac-1", "exit 0"]);
  });
  it("SegmentedControl is a named fieldset of radios with one checked", () => {
    function Host() {
      const [v, setV] = useState<"2" | "3">("3");
      return <SegmentedControl label="Max rounds" value={v} onChange={setV} options={[{ value: "2", label: "2" }, { value: "3", label: "3" }]} />;
    }
    render(<Host />);
    expect(screen.getByRole("group", { name: "Max rounds" })).toBeTruthy();
    expect((screen.getByRole("radio", { name: "3" }) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(screen.getByRole("radio", { name: "2" }));
    expect((screen.getByRole("radio", { name: "2" }) as HTMLInputElement).checked).toBe(true);
  });
  it("Switch is announced as a switch and toggles", () => {
    function Host() { const [on, setOn] = useState(false); return <Switch label="Notifications" checked={on} onChange={setOn} />; }
    render(<Host />);
    const s = screen.getByRole("switch", { name: "Notifications" });
    expect(s.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(s);
    expect(s.getAttribute("aria-checked")).toBe("true");
  });
});

describe("Sheet and AlertDialog", () => {
  it("Sheet opens modally, is named by its title and closes on Esc and on the backdrop", () => {
    const f = vi.fn();
    const { rerender } = render(<Sheet open={false} onClose={f} title="Message Backend">body</Sheet>);
    expect(document.querySelector("dialog")!.hasAttribute("open")).toBe(false);
    rerender(<Sheet open onClose={f} title="Message Backend">body</Sheet>);
    const d = document.querySelector("dialog")!;
    expect(d.hasAttribute("open")).toBe(true);
    expect(d.getAttribute("aria-labelledby")).toBeTruthy();
    expect(document.getElementById(d.getAttribute("aria-labelledby")!)!.textContent).toBe("Message Backend");
    fireEvent(d, new Event("cancel", { cancelable: true }));
    fireEvent.mouseDown(d);
    fireEvent.click(d);
    expect(f).toHaveBeenCalledTimes(2);
    rerender(<Sheet open={false} onClose={f} title="Message Backend">body</Sheet>);
    expect(d.hasAttribute("open")).toBe(false);
  });
  it("Sheet tells the parent when the browser closes it, and the backdrop needs press and release", () => {
    const f = vi.fn();
    render(<Sheet open onClose={f} title="T">body</Sheet>);
    const d = document.querySelector("dialog")!;
    fireEvent(d, new Event("close"));
    expect(f).toHaveBeenCalledTimes(1);
    fireEvent.click(d);                       // release without a press on the backdrop (a drag that began inside)
    expect(f).toHaveBeenCalledTimes(1);
    fireEvent.mouseDown(d);
    fireEvent.click(d);
    expect(f).toHaveBeenCalledTimes(2);
  });
  it("AlertDialog is an alertdialog and focuses the safe button first", () => {
    const [yes, no] = [vi.fn(), vi.fn()];
    render(<AlertDialog open destructive title="Stop Backend?" confirmLabel="Stop worker" onConfirm={yes} onCancel={no}>It will stop at its next tool call.</AlertDialog>);
    expect(document.querySelector("dialog")!.getAttribute("role")).toBe("alertdialog");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: S.cancel, hidden: true }));
    fireEvent.click(screen.getByRole("button", { name: "Stop worker", hidden: true }));
    expect(yes).toHaveBeenCalledOnce();
  });
});

describe("Toast and Skeleton", () => {
  function Host() {
    const { toasts, push, dismiss } = useToasts();
    return <><Button onClick={() => push("success", "Stopped Backend at 14:22")}>Go</Button><ToastRegion toasts={toasts} onDismiss={dismiss} /></>;
  }
  it("announces in a status region, auto-dismisses after 5 s and pauses on hover", () => {
    vi.useFakeTimers();
    render(<Host />);
    fireEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(screen.getByRole("status").textContent).toContain("Stopped Backend at 14:22");
    const toast = screen.getByText("Stopped Backend at 14:22").parentElement!;
    fireEvent.mouseEnter(toast);
    act(() => { vi.advanceTimersByTime(10000); });
    expect(screen.queryByText("Stopped Backend at 14:22")).toBeTruthy();
    fireEvent.mouseLeave(toast);
    act(() => { vi.advanceTimersByTime(5000); });
    expect(screen.queryByText("Stopped Backend at 14:22")).toBeNull();
  });
  it("an error toast is an alert and stays until dismissed", () => {
    vi.useFakeTimers();
    function Err() { const { toasts, push, dismiss } = useToasts(); return <><Button onClick={() => push("danger", "Could not write the queue entry")}>Go</Button><ToastRegion toasts={toasts} onDismiss={dismiss} /></>; }
    render(<Err />);
    fireEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(screen.getByRole("alert").textContent).toContain("Could not write the queue entry");
    act(() => { vi.advanceTimersByTime(60000); });
    expect(screen.queryByText("Could not write the queue entry")).toBeTruthy();
  });
  it("can be dismissed with its button", () => {
    render(<Host />);
    fireEvent.click(screen.getByRole("button", { name: "Go" }));
    fireEvent.click(screen.getByRole("button", { name: S.dismiss }));
    expect(screen.queryByText("Stopped Backend at 14:22")).toBeNull();
  });
  it("Skeleton appears only after 300 ms", () => {
    vi.useFakeTimers();
    const { container } = render(<Skeleton />);
    expect(container.querySelector("[aria-busy]")).toBeNull();
    act(() => { vi.advanceTimersByTime(300); });
    expect(container.querySelector("[aria-busy='true']")).toBeTruthy();
  });
});

describe("StatusDot", () => {
  it.each(Object.keys(S.status) as Array<keyof typeof S.status>)("%s has a text label and a hidden shape", (k) => {
    const { container } = render(<StatusDot status={k} />);
    expect(screen.getByText(S.status[k])).toBeTruthy();
    expect(container.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
    cleanup();
  });
});
