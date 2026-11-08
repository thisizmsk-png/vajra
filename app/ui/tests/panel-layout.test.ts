import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const base = readFileSync(join(HERE, "../components/ui/base/base.css"), "utf8");

// History and Crew stack panels in a scrolling flex column. A panel clips its content (overflow: hidden),
// so if it can shrink, rows past the fold vanish with no scrollbar. The kit panel must not shrink.
describe(".v-panel in a flex column", () => {
  it("does not shrink", () => {
    const rule = base.match(/^\.v-panel\s*\{([^}]*)\}/m);
    expect(rule, ".v-panel rule").toBeTruthy();
    expect(rule![1]).toMatch(/(^|;)\s*(flex:\s*none|flex-shrink:\s*0)\s*;/);
  });
});
