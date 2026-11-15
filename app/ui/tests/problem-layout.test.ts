import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const shell = readFileSync(join(HERE, "../src/shell.css"), "utf8");

// The problem panel is a flex column with a gap. Its "tried at" line is a live region, so it is in the page
// before the first retry, with no words. An empty flex item still costs one gap, which showed as dead space
// under Try again. jsdom has no layout, so this checks the rule: the empty line is out of the flex flow, which
// holds for any gap, and it is still rendered, so a screen reader hears it when the words arrive.
describe("the problem panel's empty tried-at line", () => {
  const rule = shell.match(/^\.v-problem \.v-checked:empty\s*\{([^}]*)\}/m);

  it("is out of the flex flow, so it adds no gap under Try again", () => {
    expect(rule, ".v-problem .v-checked:empty rule").toBeTruthy();
    expect(rule![1]).toMatch(/(^|;)\s*position:\s*absolute\s*;/);
  });

  it("does not cancel the gap with a margin tied to its size", () => {
    expect(rule![1]).not.toMatch(/margin[^;]*-/);
    expect(rule![1]).not.toMatch(/var\(--space/);
  });

  it("stays rendered, so its status role still announces", () => {
    expect(rule![1]).not.toMatch(/display:\s*none|visibility:\s*hidden/);
  });
});
