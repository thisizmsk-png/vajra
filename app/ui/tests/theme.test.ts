import { describe, expect, it } from "vitest";
import { followSystemTheme, systemTheme } from "../src/theme";

function fakeMq(matches: boolean) {
  const listeners = new Set<() => void>();
  return {
    matches,
    set(v: boolean) {
      this.matches = v;
      listeners.forEach((cb) => cb());
    },
    addEventListener: (_: "change", cb: () => void) => void listeners.add(cb),
    removeEventListener: (_: "change", cb: () => void) => void listeners.delete(cb),
  };
}

describe("theme follows the OS (prefers-color-scheme)", () => {
  it("is dark when there is no light preference or no media query support", () => {
    expect(systemTheme({ matches: false })).toBe("dark");
    expect(systemTheme(undefined)).toBe("dark");
  });

  it("is light when the OS asks for light", () => {
    expect(systemTheme({ matches: true })).toBe("light");
  });

  it("sets data-theme on the root and follows OS changes until stopped", () => {
    const root = document.createElement("html");
    const mq = fakeMq(true);
    const stop = followSystemTheme(root, mq);
    expect(root.dataset.theme).toBe("light");
    mq.set(false);
    expect(root.dataset.theme).toBe("dark");
    stop();
    mq.set(true);
    expect(root.dataset.theme).toBe("dark");
  });

  it("falls back to dark with no media query", () => {
    const root = document.createElement("html");
    followSystemTheme(root, undefined);
    expect(root.dataset.theme).toBe("dark");
  });
});
