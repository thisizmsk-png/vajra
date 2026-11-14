import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { FindRepos } from "../src/shell/FindRepos";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
});

const W = S.frame.welcome;

describe("FindRepos", () => {
  it("says so when the scan fails, and does not look like finding none", async () => {
    setTransport(fakeIpc({ project_discover: () => { throw { error: "io_error", detail: "x" }; } }).transport);
    render(<FindRepos onAdded={() => {}} />);
    expect((await screen.findByRole("alert")).textContent).toContain(W.scanFailed);
  });

  it("looks again on the button, and offers the repositories it then finds", async () => {
    let n = 0;
    setTransport(fakeIpc({
      project_discover: () => {
        if (n++ === 0) throw { error: "io_error", detail: "x" };
        return { found: [{ root: "/r/a", name: "a", worktrees: 0 }], truncated: false };
      },
    }).transport);
    render(<FindRepos onAdded={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: W.scanAgain }));
    await waitFor(() => expect(screen.getByRole("button", { name: W.offerShort(1) })).toBeTruthy());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps the alert and the button in the document while it looks again, so focus is not lost", async () => {
    let release: () => void = () => {};
    let n = 0;
    setTransport(fakeIpc({
      project_discover: () => {
        if (n++ === 0) throw { error: "io_error", detail: "x" };
        return new Promise((r) => { release = () => r({ found: [], truncated: false }); });
      },
    }).transport);
    render(<FindRepos onAdded={() => {}} />);
    const btn = await screen.findByRole("button", { name: W.scanAgain });
    btn.focus();
    fireEvent.click(btn);
    await waitFor(() => expect(screen.getByRole("button", { name: W.scanning })).toBeTruthy());
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(screen.getByRole("button", { name: W.scanning }).hasAttribute("disabled")).toBe(true);
    release();
  });

  it("stays silent when the scan works and finds nothing", async () => {
    const f = fakeIpc({ project_discover: { found: [], truncated: false } });
    setTransport(f.transport);
    const { container } = render(<FindRepos onAdded={() => {}} />);
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "project_discover")).toBe(true));
    expect(container.textContent).toBe("");
  });
});
