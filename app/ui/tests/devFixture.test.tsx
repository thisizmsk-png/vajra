import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { installFixture } from "../src/devFixture";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { Home } from "../src/screens/home/Home";
import { S } from "../strings";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

describe("fixture mode (dev ?fixture)", () => {
  it("shows stream rows for a worker instead of not_in_fixture", async () => {
    installFixture();
    window.location.hash = "#/crew?slug=deliver-20261106-2140&id=20261106-2140";
    render(<Crew />);
    // It opens on the active worker's Live tab: its messages and steps.
    const feed = await screen.findByRole("log", { name: S.crew.live.label });
    await waitFor(() => expect(feed.textContent).toContain("test/rateLimit.redis.test.ts"));
    expect(feed.textContent).toContain("All three pass.");
    fireEvent.click(await screen.findByRole("tab", { name: S.crew.tabs.stream }));
    const log = await screen.findByRole("log", { name: S.crew.streamLabel });
    await waitFor(() => expect(log.textContent).toContain("Write test/rateLimit.redis.test.ts"));
    expect(screen.queryByText(/not_in_fixture/)).toBeNull();
  });

  it("shows the gap notice for the worker whose first lines were lost", async () => {
    installFixture();
    window.location.hash = "#/crew?slug=deliver-20261106-2140&id=20261106-2140";
    render(<Crew />);
    fireEvent.click((await screen.findByText("Backend worker 1")).closest("[role=treeitem]") as HTMLElement);
    fireEvent.click(await screen.findByRole("tab", { name: S.crew.tabs.stream }));
    await screen.findByRole("log", { name: S.crew.streamLabel });
    await waitFor(() => expect(screen.getByText(S.crew.windowGap)).toBeTruthy());
  });

  it("has a first run whose checks stop a start, so that state can be looked at", async () => {
    installFixture("blocked");
    render(<Home />);
    expect(await screen.findByText(S.home.startNeedsChecks(1))).toBeTruthy();
    expect(screen.getByRole("button", { name: S.home.startButton }).getAttribute("aria-disabled")).toBe("true");
  });
});
