// Settings (and the accounts and project-agents panels in it) is a lazy chunk: importing the screen registry and showing
// the other screens never loads it, and it still renders once opened. The idle memory gate counts every kB of idle JS.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const loaded = vi.hoisted(() => ({ count: 0 }));
vi.mock("../src/screens/settings/Settings", () => {
  loaded.count++;
  return { Settings: () => <p>settings chunk</p> };
});

afterEach(cleanup);

describe("Settings is a lazy chunk", () => {
  it("importing the registry does not load it; opening Settings does, and it renders", async () => {
    const { REGISTRY } = await import("../src/screens/registry");
    expect(loaded.count).toBe(0);
    const Settings = REGISTRY.settings;
    render(<Settings />);
    expect(await screen.findByText("settings chunk")).toBeTruthy();
    expect(loaded.count).toBe(1);
  });
});
