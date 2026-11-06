import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../src/Shell";
import { REGISTRY } from "../src/screens/registry";
import { S } from "../strings";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

describe("Shell", () => {
  it("shows home by default and marks the current nav link", () => {
    render(<Shell />);
    expect(screen.getByRole("link", { name: S.nav.home }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("link", { name: S.nav.run }).getAttribute("aria-current")).toBeNull();
  });

  it("follows the hash", () => {
    window.location.hash = "#/history";
    render(<Shell />);
    expect(screen.getByRole("heading", { name: S.nav.history })).toBeTruthy();
    expect(screen.getByRole("link", { name: S.nav.history }).getAttribute("aria-current")).toBe("page");
  });

  it("contains a throwing screen and recovers when you leave it", () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const orig = REGISTRY.crew;
    REGISTRY.crew = () => { throw new Error("boom"); };
    window.location.hash = "#/crew";
    render(<Shell />);
    expect(screen.getByRole("alert").textContent).toContain(S.shell.errorTitle);
    expect(screen.getByRole("link", { name: S.nav.home })).toBeTruthy(); // nav still works
    fireEvent.click(screen.getByRole("link", { name: S.nav.home }));
    window.location.hash = "#/home";
    REGISTRY.crew = orig;
    err.mockRestore();
  });
});
