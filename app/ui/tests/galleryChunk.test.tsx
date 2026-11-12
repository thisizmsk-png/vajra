// The component gallery is a developer page. It must stay out of the start bundle (the idle memory gate counts every
// kB), and its address must still open it.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "../src/App";

const here = dirname(fileURLToPath(import.meta.url));

describe("the component gallery", () => {
  afterEach(() => {
    cleanup();
    location.hash = "";
    document.documentElement.dataset.theme = "";
  });

  it("is not imported at start by the app entry or the shell", () => {
    for (const file of ["../src/App.tsx", "../src/Shell.tsx", "../src/main.tsx"]) {
      const source = readFileSync(join(here, file), "utf8");
      expect(source, file).not.toMatch(/^import[^\n]*from "\.\/Gallery(Desktop)?";/m);
    }
    expect(readFileSync(join(here, "../src/App.tsx"), "utf8")).toContain('lazy(() => import("./Gallery")');
  });

  it("still opens at its developer address", async () => {
    location.hash = "#kit";
    render(<App />);
    // The gallery sets the page theme when it mounts; the shell is not what rendered.
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"));
    expect(document.querySelector(".v-shell")).toBeNull();
  });
});
