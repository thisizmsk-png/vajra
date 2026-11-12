// A preflight row's one-click fix (6c-2): with no git on the Mac, "Install developer tools" opens Apple's installer only
// when clicked, then says what to do next. Rows without an action are unchanged.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { PreflightList } from "../src/screens/PreflightList";
import { fakeIpc } from "./fakeIpc";

afterEach(() => { cleanup(); setTransport(); });

const row = { name: "Git settings for agents", ok: false, detail: "Agents are off: git isn't installed.", fix: "", action: "install_dev_tools" as const, action_label: "Install developer tools", action_note: "When the install finishes, restart Vajra." };

describe("preflight row action", () => {
  it("runs only on click, then shows the next step", async () => {
    const f = fakeIpc({ install_dev_tools: { started: true } });
    setTransport(f.transport);
    render(<PreflightList rows={[row]} />);
    expect(f.calls).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Install developer tools" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("When the install finishes, restart Vajra."));
    expect(f.calls.map((c) => c.cmd)).toEqual(["install_dev_tools"]);
  });

  it("a row with no action shows no button", () => {
    setTransport(fakeIpc().transport);
    render(<PreflightList rows={[{ name: "jq", ok: false, detail: "missing", fix: "brew install jq" }]} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("brew install jq")).toBeTruthy();
  });
});
