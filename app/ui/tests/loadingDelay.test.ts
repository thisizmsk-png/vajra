// Loading rows must not flash: a view is its own chunk and usually arrives within a few milliseconds of its first open
// (screens/registry.tsx), so the rows stay hidden for 100 ms and only a slow load ever shows them.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../src/shell.css"), "utf8");

describe("loading rows", () => {
  it("stay hidden for 100 ms, then show", () => {
    expect(css).toContain(".v-loading { visibility: hidden; animation: v-loading-show 0s linear 100ms forwards; }");
    expect(css).toContain("@keyframes v-loading-show { to { visibility: visible; } }");
  });
});
