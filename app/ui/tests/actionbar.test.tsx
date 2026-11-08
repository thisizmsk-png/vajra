import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Kbd, ariaShortcut } from "../components/ui/base";
import { ActionBar, chordMatches } from "../components/ui/desktop";

afterEach(cleanup);

const ev = (key: string, mods: Partial<Record<"metaKey" | "shiftKey" | "altKey" | "ctrlKey", boolean>> = {}, target: Element = document.body) =>
  ({ key, metaKey: false, shiftKey: false, altKey: false, ctrlKey: false, ...mods, target }) as unknown as KeyboardEvent;

describe("Kbd", () => {
  it("names symbol keys for screen readers", () => {
    render(<Kbd keys={["⌘", "↵"]} />);
    expect(screen.getByLabelText("Command Return")).toBeTruthy();
  });
  it("is hidden from the name when decorative, and the chord has an aria-keyshortcuts form", () => {
    render(<button aria-keyshortcuts={ariaShortcut(["⌘", "↵"])}>Start run <Kbd keys={["⌘", "↵"]} decorative /></button>);
    const b = screen.getByRole("button", { name: "Start run" });
    expect(b.getAttribute("aria-keyshortcuts")).toBe("Meta+Enter");
  });
});

describe("chordMatches", () => {
  it("matches the exact modifiers and key", () => {
    expect(chordMatches(ev("Enter", { metaKey: true }), ["⌘", "↵"])).toBe(true);
    expect(chordMatches(ev("Enter"), ["⌘", "↵"])).toBe(false);
    expect(chordMatches(ev("Enter", { metaKey: true, shiftKey: true }), ["⌘", "↵"])).toBe(false);
    expect(chordMatches(ev("m"), ["M"])).toBe(true);
  });
  it("never fires a bare key while typing in a field", () => {
    const input = document.createElement("input");
    expect(chordMatches(ev("m", {}, input), ["M"])).toBe(false);
    expect(chordMatches(ev("Enter", { metaKey: true }, input), ["⌘", "↵"])).toBe(true);
  });
});

describe("ActionBar", () => {
  it("shows the context and the actions, primary first, and runs one on click", () => {
    const run = vi.fn();
    render(<ActionBar label="Actions" context={<b>Backend</b>} actions={[
      { id: "m", label: "Message", keys: ["M"], run: () => {} },
      { id: "a", label: "Approve plan", keys: ["⌘", "↵"], primary: true, run },
    ]} />);
    const bar = screen.getByRole("contentinfo", { name: "Actions" });
    expect(bar.textContent).toContain("Backend");
    const buttons = screen.getAllByRole("button");
    expect(buttons.map((b) => b.textContent?.replace(/[⌘↵M]+$/, ""))).toEqual(["Approve plan", "Message"]);
    fireEvent.click(screen.getByRole("button", { name: "Approve plan" }));
    expect(run).toHaveBeenCalledTimes(1);
  });
  it("runs an action from its chord, and not when it is disabled", () => {
    const run = vi.fn();
    const off = vi.fn();
    render(<ActionBar label="Actions" actions={[
      { id: "a", label: "Approve plan", keys: ["⌘", "↵"], run },
      { id: "s", label: "Stop", keys: ["⌘", "."], disabled: true, run: off },
    ]} />);
    fireEvent.keyDown(window, { key: "Enter", metaKey: true });
    fireEvent.keyDown(window, { key: ".", metaKey: true });
    expect(run).toHaveBeenCalledTimes(1);
    expect(off).not.toHaveBeenCalled();
  });
});
