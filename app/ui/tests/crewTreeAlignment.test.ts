import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const HERE = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(HERE, "../src/shell.css"), "utf8");
const tree = readFileSync(join(HERE, "../components/ui/desktop/CrewTree.tsx"), "utf8");

// jsdom has no layout, so the alignment rule is held at its source: a row's icon starts at
// paddingLeft + twisty slot, and a leaf and a branch at one level only line up if both rows have the same slot.
describe("Crew tree alignment under 1200 px", () => {
  const narrow = css.slice(css.indexOf("@media (max-width: 1199px)"));

  it("never hides the empty chevron slot of a leaf row", () => {
    expect(narrow).not.toMatch(/\.v-tree__twisty:empty\s*\{[^}]*display:\s*none/);
  });

  it("gives every row the same slot, one indent step wide, so a leaf's icon is under its parent's label", () => {
    const step = Number(/\(r\.level - 1\) \* (\d+)/.exec(tree)?.[1]);
    const slot = Number(/\.v-crew__tree \.v-tree__twisty\s*\{\s*min-width:\s*(\d+)px/.exec(narrow)?.[1]);
    expect(step).toBeGreaterThan(0);
    expect(slot).toBe(step);
  });
});
