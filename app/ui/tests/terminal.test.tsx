import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport, TERM_WRITE_CHUNK } from "../src/ipc";
import { createInput } from "../src/terminal/input";
import { TerminalDock, openTerminalHere } from "../src/terminal/TerminalDock";
import { S } from "../strings";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

const PLACE = { cwd: "/work/app", label: "app" };
const ctrlTick = (target: Element | Window = window) => fireEvent.keyDown(target, { key: "`", code: "Backquote", ctrlKey: true });

beforeEach(() => {
  FakeXterm.all = [];
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} disconnect() {} };
});
afterEach(() => {
  cleanup();
  setTransport();
});

/** Mount with a fake backend and open the first terminal with the shortcut; resolves once the shell is running. */
async function opened(over: Parameters<typeof fakeTerm>[0] = {}, place = PLACE) {
  const f = fakeTerm(over);
  setTransport(f.transport);
  render(<TerminalDock defaultPlace={place} />);
  act(() => void ctrlTick());
  await waitFor(() => expect(FakeXterm.all.length).toBe(1));
  await waitFor(() => expect(f.named("term_open").length).toBe(1));
  await act(async () => {});
  return { f, x: FakeXterm.all[0] };
}

describe("createInput", () => {
  it("sends pieces in order, none over the chunk size, and never splits a surrogate pair", async () => {
    const sent: string[] = [];
    const i = createInput({ send: async (d) => void sent.push(d), onDrop: vi.fn(), onGone: vi.fn() });
    const text = "a".repeat(TERM_WRITE_CHUNK - 1) + "😀" + "b".repeat(10);
    i.push(text);
    i.push("!");
    await i.flush();
    expect(sent.join("")).toBe(text + "!");
    expect(sent.every((s) => s.length <= TERM_WRITE_CHUNK)).toBe(true);
    expect(sent[0].endsWith("a")).toBe(true);
  });
  it("retries a full queue, then drops the piece and says so", async () => {
    let calls = 0;
    const onDrop = vi.fn();
    const i = createInput({ send: async () => { calls++; throw { error: "input_queue_full" }; }, onDrop, onGone: vi.fn(), retries: 3, sleep: async () => {} });
    i.push("x");
    await i.flush();
    expect(calls).toBe(4);
    expect(onDrop).toHaveBeenCalledTimes(1);
  });
  it("succeeds when the shell starts reading again", async () => {
    let n = 0;
    const sent: string[] = [];
    const onDrop = vi.fn();
    const i = createInput({ send: async (d) => { if (n++ < 2) throw { error: "input_queue_full" }; sent.push(d); }, onDrop, onGone: vi.fn(), sleep: async () => {} });
    i.push("hi");
    await i.flush();
    expect(sent).toEqual(["hi"]);
    expect(onDrop).not.toHaveBeenCalled();
  });
  it("stops for good when the terminal is gone", async () => {
    const onGone = vi.fn();
    const send = vi.fn(async () => { throw { error: "not_found" }; });
    const i = createInput({ send, onDrop: vi.fn(), onGone });
    i.push("a");
    i.push("b");
    await i.flush();
    expect(send).toHaveBeenCalledTimes(1);
    expect(onGone).toHaveBeenCalledTimes(1);
  });
});

describe("TerminalDock", () => {
  it("renders nothing until it is opened", () => {
    setTransport(fakeTerm().transport);
    const { container } = render(<TerminalDock defaultPlace={PLACE} />);
    expect(container.innerHTML).toBe("");
  });

  it("Control+backtick opens a terminal in the default place, at the view's size", async () => {
    const { f, x } = await opened();
    expect(f.named("term_open")[0].args).toEqual({ cwd: "/work/app", rows: 24, cols: 80 });
    expect(screen.getByRole("region", { name: S.terminal.region })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "app" }).getAttribute("aria-selected")).toBe("true");
    expect(x.textarea.getAttribute("aria-label")).toBe(S.terminal.inputLabel);
    expect(x.focused).toBeGreaterThan(0);
  });

  it("hiding keeps the shell running and returns focus; showing again does not open another", async () => {
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    outside.focus();
    const f = fakeTerm();
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => void ctrlTick());
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    act(() => void ctrlTick());
    expect(document.querySelector("section.v-term")!.hasAttribute("hidden")).toBe(true);
    expect(document.activeElement).toBe(outside);
    expect(f.named("term_close").length).toBe(0);
    act(() => void ctrlTick());
    expect(f.named("term_open").length).toBe(1);
    expect(FakeXterm.all[0].disposed).toBe(false);
    outside.remove();
  });

  it("writes output to xterm and acknowledges it in 64 KiB steps", async () => {
    const { f, x } = await opened();
    const out = f.streams[0];
    act(() => out.send(new Uint8Array(32 * 1024).buffer));
    expect(f.named("term_ack").length).toBe(0);
    act(() => out.send(new Uint8Array(32 * 1024).buffer));
    expect(f.named("term_ack")).toEqual([{ cmd: "term_ack", args: { id: 1, bytes: 64 * 1024 } }]);
    act(() => out.send(f.bytes("hello").slice().buffer as ArrayBuffer));
    expect(x.text().endsWith("hello")).toBe(true);
  });

  it("sends typed text and chunks a big paste in order", async () => {
    const { f, x } = await opened();
    act(() => x.type("ls\r"));
    const big = "p".repeat(TERM_WRITE_CHUNK * 2 + 5);
    act(() => x.type(big));
    await waitFor(() => expect(f.named("term_write").length).toBe(4));
    const sent = f.named("term_write").map((c) => c.args!.data as string);
    expect(sent[0]).toBe("ls\r");
    expect(sent.slice(1).join("")).toBe(big);
    expect(sent.every((s) => s.length <= TERM_WRITE_CHUNK)).toBe(true);
    expect(f.named("term_write").every((c) => c.args!.id === 1)).toBe(true);
  });

  it("passes a resize to the core", async () => {
    const { f, x } = await opened();
    act(() => x.resize(40, 132));
    expect(f.named("term_resize")[0].args).toEqual({ id: 1, rows: 40, cols: 132 });
  });

  it("copy and paste: Ctrl+Shift+C copies the selection, Ctrl+Shift+V pastes the clipboard, Ctrl+C is left to the shell", async () => {
    const { x } = await opened();
    const clip = { writeText: vi.fn(async () => {}), readText: vi.fn(async () => "pasted text") };
    Object.defineProperty(navigator, "clipboard", { value: clip, configurable: true });
    x.selection = "chosen";
    const key = (o: Partial<KeyboardEventInit>) => x.keyHandler!(new KeyboardEvent("keydown", o));
    expect(key({ ctrlKey: true, shiftKey: true, code: "KeyC", key: "C" })).toBe(false);
    expect(clip.writeText).toHaveBeenCalledWith("chosen");
    expect(key({ ctrlKey: true, shiftKey: true, code: "KeyV", key: "V" })).toBe(false);
    await waitFor(() => expect(x.pasted).toEqual(["pasted text"]));
    expect(key({ ctrlKey: true, code: "KeyC", key: "c" })).toBe(true);
  });

  it("shows an ended shell, announces it, and starts a fresh one on request", async () => {
    const { f } = await opened();
    act(() => f.streams[0].send({ exit: 3 }));
    expect(screen.getByText(S.terminal.ended(3))).toBeTruthy();
    expect(screen.getByRole("tab", { name: S.terminal.endedTab("app") })).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe(S.terminal.announceEnded("app"));
    fireEvent.click(screen.getByRole("button", { name: S.terminal.restart }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    expect(f.named("term_close")[0].args).toEqual({ id: 1 });
  });

  it("explains a refused open and retries on request", async () => {
    let fail = true;
    const f = fakeTerm({ term_open: (a?: Record<string, unknown>) => (fail ? Object.assign(new Error("x"), {}) : { id: 9, cwd: a?.cwd, root: a?.cwd, rows: 24, cols: 80, alive: true }) });
    // A rejected command carries the core's {error, detail}.
    const open = f.transport.channel!;
    f.transport.channel = async (c, a, m) => {
      if (fail) { f.calls.push({ cmd: c, args: a }); throw { error: "cwd_not_allowed", detail: "not a root" }; }
      return open(c, a, m);
    };
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => void ctrlTick());
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(S.terminal.errors.cwd_not_allowed);
    fail = false;
    fireEvent.click(screen.getByRole("button", { name: S.terminal.retry }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(FakeXterm.all.length).toBe(2);
  });

  it("a backend that cannot stream says so instead of hanging", async () => {
    const f = fakeTerm();
    delete f.transport.channel;
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => void ctrlTick());
    expect((await screen.findByRole("alert")).textContent).toContain(S.terminal.errors.no_terminal_transport);
  });

  it("Terminal here opens a tab per place and focuses an existing one", async () => {
    const f = fakeTerm();
    setTransport(f.transport);
    render(<TerminalDock />);
    act(() => openTerminalHere({ cwd: "/work/a", label: "a" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    act(() => openTerminalHere({ cwd: "/work/b", label: "b" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    expect(screen.getByRole("tab", { name: "b" }).getAttribute("aria-selected")).toBe("true");
    act(() => openTerminalHere({ cwd: "/work/a", label: "a" }));
    expect(f.named("term_open").length).toBe(2);
    expect(screen.getByRole("tab", { name: "a" }).getAttribute("aria-selected")).toBe("true");
  });

  it("closing a tab closes its shell and selects a neighbour; the last one hides the dock", async () => {
    const f = fakeTerm();
    setTransport(f.transport);
    render(<TerminalDock />);
    act(() => openTerminalHere({ cwd: "/work/a", label: "a" }));
    act(() => openTerminalHere({ cwd: "/work/b", label: "b" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: S.terminal.closeTab("b") }));
    await waitFor(() => expect(f.named("term_close")).toEqual([{ cmd: "term_close", args: { id: 2 } }]));
    expect(screen.getByRole("tab", { name: "a" }).getAttribute("aria-selected")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: S.terminal.closeTab("a") }));
    await waitFor(() => expect(f.named("term_close").length).toBe(2));
    expect(screen.queryByRole("tab")).toBeNull();
  });

  it("picks up terminals that outlived the webview by attaching to them", async () => {
    const f = fakeTerm({ term_list: { terminals: [{ id: 7, cwd: "/work/app/.claude/worktrees/w1", root: "/work/app", rows: 24, cols: 80, alive: true }] } });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    await waitFor(() => expect(f.named("term_attach")[0]?.args).toEqual({ id: 7 }));
    act(() => ctrlTick());
    expect(screen.getByRole("tab", { name: "app/w1" })).toBeTruthy();
    expect(f.named("term_open").length).toBe(0);
  });

  it("attaching under StrictMode (setup, cleanup, setup) does not close the terminal the second setup attached", async () => {
    const f = fakeTerm({ term_list: { terminals: [{ id: 7, cwd: "/work/app", root: "/work/app", rows: 24, cols: 80, alive: true }] } });
    setTransport(f.transport);
    render(
      <StrictMode>
        <TerminalDock defaultPlace={PLACE} />
      </StrictMode>,
    );
    await waitFor(() => expect(f.named("term_attach").length).toBeGreaterThanOrEqual(2)); // the disposed run and the live one
    await act(async () => {});
    expect(f.named("term_close")).toEqual([]);
    act(() => ctrlTick());
    expect(screen.getByRole("tab", { name: "app" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.queryByText(S.terminal.ended(0))).toBeNull();
  });

  it("the empty state says what a terminal is and offers to open one", async () => {
    const f = fakeTerm();
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => openTerminalHere(PLACE));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: S.terminal.closeTab("app") }));
    // closing the last tab hides; showing it again with the shortcut finds none and opens one in the default place
    act(() => ctrlTick());
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    expect(S.terminal.emptyHint).toMatch(/not sandboxed/);
  });

  it("tabs are a labelled tablist that arrow keys walk", async () => {
    const f = fakeTerm();
    setTransport(f.transport);
    render(<TerminalDock />);
    act(() => openTerminalHere({ cwd: "/work/a", label: "a" }));
    act(() => openTerminalHere({ cwd: "/work/b", label: "b" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    const list = screen.getByRole("tablist", { name: S.terminal.tabs });
    expect(screen.getByRole("tab", { name: "a" }).getAttribute("tabindex")).toBe("-1");
    expect(screen.getByRole("tab", { name: "b" }).getAttribute("tabindex")).toBe("0");
    fireEvent.keyDown(list, { key: "ArrowLeft" });
    expect(screen.getByRole("tab", { name: "a" }).getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(list, { key: "End" });
    expect(screen.getByRole("tab", { name: "b" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("region", { name: S.terminal.region }).getAttribute("aria-describedby")).toBe("v-term-hint");
    expect(screen.getByText(S.terminal.leaveHint)).toBeTruthy();
  });

  it("closing the view (unmount) closes the shell", async () => {
    const { f, x } = await opened();
    cleanup();
    expect(x.disposed).toBe(true);
    expect(f.named("term_close")).toEqual([{ cmd: "term_close", args: { id: 1 } }]);
  });

  it("tells the user when typing was dropped because the shell is not reading", async () => {
    const f = fakeTerm({ term_write: () => { throw { error: "input_queue_full" }; } });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => void ctrlTick());
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    await act(async () => {});
    vi.useFakeTimers();
    act(() => FakeXterm.all[0].type("x"));
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    vi.useRealTimers();
    expect(screen.getByRole("status").textContent).toBe(S.terminal.busy);
  });
});
