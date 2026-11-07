import { StrictMode } from "react";
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Gallery } from "../src/Gallery";

// Regression: under StrictMode the simulated effect cleanup must not leave the gallery on the OS theme while the
// toggle reads Dark. The OS here is light, so a render-time or mis-ordered write would show up as "light".
describe("Gallery theme under StrictMode", () => {
  afterEach(() => {
    cleanup();
    delete (window as { matchMedia?: unknown }).matchMedia;
    document.documentElement.dataset.theme = "";
  });

  it("opens on the toggle's theme (dark) even when the OS is light", () => {
    window.matchMedia = ((query: string) => ({
      matches: query === "(prefers-color-scheme: light)",
      addEventListener: () => {},
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;
    render(<StrictMode><Gallery /></StrictMode>);
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});
