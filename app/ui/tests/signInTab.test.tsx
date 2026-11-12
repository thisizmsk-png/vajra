// ADR-17 rule 6: Sign in opens the CLI's own login in a terminal tab. The tab asks the core by harness id only, and it
// is never saved in the tab layout (a restart must not re-run a sign-in).
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { S } from "../strings";
import { LAYOUT_KEY, TerminalDock, onSignInEnded, openSignIn, openTerminalHere } from "../src/terminal/TerminalDock";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

const store = new Map<string, string>();
vi.stubGlobal("localStorage", { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k), clear: () => store.clear() });

const APP = { cwd: "/work/app", label: "app" };
const signInInfo = { id: 41, pid: 4242, cwd: "/tmp/vajra-signin-abc", root: "/tmp/vajra-signin-abc", rows: 24, cols: 80, alive: true };

beforeEach(() => {
  localStorage.clear();
  FakeXterm.all = [];
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} disconnect() {} };
});
afterEach(() => {
  cleanup();
  setTransport();
});

function mount(place: typeof APP | null = APP) {
  const f = fakeTerm({ setup_sign_in: () => signInInfo });
  setTransport(f.transport);
  render(<TerminalDock defaultPlace={place ?? undefined} />);
  return f;
}

describe("sign-in tab", () => {
  it("opens through setup_sign_in with the harness id and nothing else", async () => {
    const f = mount();
    act(() => openSignIn({ id: "claude", label: "Sign in: Claude Code" }));
    await waitFor(() => expect(f.named("setup_sign_in").length).toBe(1));
    const args = f.named("setup_sign_in")[0].args ?? {};
    expect(Object.keys(args).sort()).toEqual(["cols", "id", "rows"]);
    expect(args.id).toBe("claude");
    expect(f.named("term_open").length).toBe(0);
  });

  it("focuses the open sign-in tab for the same harness instead of starting a second one", async () => {
    const f = mount();
    act(() => openSignIn({ id: "claude", label: "Sign in: Claude Code" }));
    await waitFor(() => expect(f.named("setup_sign_in").length).toBe(1));
    act(() => openSignIn({ id: "claude", label: "Sign in: Claude Code" }));
    await new Promise((r) => setTimeout(r, 20));
    expect(f.named("setup_sign_in").length).toBe(1);
  });

  it("is never written to the saved tab layout", async () => {
    const f = mount();
    act(() => openTerminalHere(APP));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    act(() => openSignIn({ id: "codex", label: "Sign in: Codex CLI" }));
    await waitFor(() => expect(f.named("setup_sign_in").length).toBe(1));
    const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    expect(saved.tabs).toEqual([{ cwd: "/work/app", label: "app", kind: "shell" }]);
  });

  it("opens + from a sign-in tab in the default folder, never in the sign-in folder", async () => {
    const f = mount();
    act(() => openSignIn({ id: "claude", label: "Sign in: Claude Code" }));
    await waitFor(() => expect(f.named("setup_sign_in").length).toBe(1));
    fireEvent.click(screen.getByRole("button", { name: S.terminal.newTab }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    expect(f.named("term_open")[0].args?.cwd).toBe("/work/app");
  });

  it("disables + on a sign-in tab when there is no default folder", async () => {
    const f = mount(null);
    act(() => openSignIn({ id: "claude", label: "Sign in: Claude Code" }));
    await waitFor(() => expect(f.named("setup_sign_in").length).toBe(1));
    expect(screen.getByRole("button", { name: S.terminal.newTab })).toHaveProperty("disabled", true);
  });

  it("says which harness finished signing in when its CLI exits, and a plain shell ending says nothing (ADR-17 rule 6)", async () => {
    const ended: string[] = [];
    const off = onSignInEnded((id) => ended.push(id));
    try {
      const f = fakeTerm({ setup_sign_in: () => signInInfo });
      setTransport(f.transport);
      render(<TerminalDock defaultPlace={APP} />);
      act(() => openTerminalHere(APP));
      act(() => openSignIn({ id: "codex", label: "Sign in: Codex CLI" }));
      await waitFor(() => expect(f.streams.length).toBe(2));
      const shell = f.streams.find((s) => s.cmd === "term_open")!;
      const signIn = f.streams.find((s) => s.cmd === "setup_sign_in")!;
      act(() => shell.send({ exit: 0 }));
      expect(ended).toEqual([]);
      act(() => signIn.send({ exit: 0 }));
      expect(ended).toEqual(["codex"]);
    } finally {
      off();
    }
  });

  it("after a webview reload, a live sign-in comes back as a sign-in tab: never saved, still reports its end, Restart signs in again", async () => {
    const ended: string[] = [];
    const off = onSignInEnded((id) => ended.push(id));
    try {
      const live = [
        { id: 3, cwd: "/work/app", root: "/work/app", rows: 24, cols: 80, alive: true },
        { ...signInInfo, id: 9, sign_in: { id: "codex", name: "Codex CLI" } },
      ];
      const f = fakeTerm({ term_list: { terminals: live }, setup_sign_in: () => signInInfo });
      setTransport(f.transport);
      render(<TerminalDock defaultPlace={APP} />);
      expect(await screen.findByRole("tab", { name: "Sign in: Codex CLI", hidden: true })).toBeTruthy();
      act(() => void fireEvent.keyDown(window, { key: "`", code: "Backquote", ctrlKey: true }));
      await waitFor(() => expect(f.named("term_attach").map((c) => c.args?.id).sort()).toEqual([3, 9]));
      const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
      expect(saved.tabs).toEqual([{ cwd: "/work/app", label: "app", kind: "shell" }]);
      const stream = f.streams.find((s) => s.cmd === "term_attach" && s.id === 9)!;
      act(() => stream.send({ exit: 0 }));
      expect(ended).toEqual(["codex"]);
      fireEvent.click(await screen.findByRole("tab", { name: S.terminal.endedTab("Sign in: Codex CLI") }));
      fireEvent.click(await screen.findByRole("button", { name: S.terminal.restart }));
      await waitFor(() => expect(f.named("setup_sign_in").map((c) => c.args?.id)).toEqual(["codex"]));
      expect(f.named("term_open").length).toBe(0);
    } finally {
      off();
    }
  });
});
