// Gap 10: a bare error code never reaches the welcome, a chord is one keycap, and the palette has its two sections.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { Kbd } from "../components/ui/base";
import { chordText } from "../components/ui/base/Kbd";
import { CommandPalette } from "../components/ui/desktop/CommandPalette";
import { reason } from "../src/welcome/model";
import { S } from "../strings";

beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
});
afterEach(cleanup);

describe("welcome reason", () => {
  it("keeps a sentence the core wrote", () => {
    expect(reason({ error: "io_error", detail: "Permission denied on the home folder." })).toBe("Permission denied on the home folder.");
  });
  it("turns a bare code into words, or into nothing when the code is not known", () => {
    expect(reason({ error: "unknown_command", detail: "project_discover" })).toBe(S.states.unknownCommand);
    expect(reason({ error: "not_in_fixture", detail: "project_discover" })).toBe(S.states.unknownCommand);
    expect(reason({ error: "something_new", detail: "project_discover" })).toBe("");
    expect(S.welcome.folder.failedBody(reason({ error: "something_new", detail: "project_discover" }))).not.toContain("project_discover");
  });
});

describe("Kbd", () => {
  it("writes a chord as one keycap and keeps the spoken names", () => {
    const { container } = render(<Kbd keys={["⌘", "1"]} />);
    const caps = container.querySelectorAll(".v-kbd");
    expect(caps).toHaveLength(1);
    expect(caps[0].textContent).toBe("⌘1");
    expect(container.querySelector(".v-kbd-group")?.getAttribute("aria-label")).toBe("Command 1");
  });
  it("spaces a word key from its neighbours", () => {
    expect(chordText(["⌘", "⇧", "P"])).toBe("⌘⇧P");
    expect(chordText(["⌘", "esc"])).toBe("⌘ esc");
  });
});

describe("command palette sections", () => {
  const cmds = [
    { id: "a", label: "Open Settings", run: () => {} },
    { id: "b", label: "Go to project api", group: "go" as const, run: () => {} },
    { id: "c", label: "Show Board", group: "go" as const, run: () => {} },
  ];
  it("lists places under Go to, then Actions, and arrows follow that order", () => {
    render(<CommandPalette open onClose={() => {}} commands={cmds} />);
    const groups = screen.getAllByRole("group");
    expect(groups.map((g) => g.getAttribute("aria-labelledby") && document.getElementById(g.getAttribute("aria-labelledby")!)?.textContent)).toEqual([S.palette.goTo, S.palette.actions]);
    expect(within(groups[0]).getAllByRole("option").map((o) => o.textContent)).toEqual(["Go to project api", "Show Board"]);
    const opts = screen.getAllByRole("option");
    expect(opts[0].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "ArrowDown" });
    expect(screen.getAllByRole("option")[2].textContent).toBe("Open Settings");
    expect(screen.getAllByRole("option")[2].getAttribute("aria-selected")).toBe("true");
  });
  it("a search drops the sections and ranks one list", () => {
    render(<CommandPalette open onClose={() => {}} commands={cmds} />);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "board" } });
    expect(screen.queryAllByRole("group")).toHaveLength(0);
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["Show Board"]);
  });
});
