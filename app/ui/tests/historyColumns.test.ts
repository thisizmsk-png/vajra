import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../src/shell.css"), "utf8");
const width = (selector: string) => Number(new RegExp(`${selector.replace(/[().]/g, "\\$&")}\\s*\\{[^}]*?width:\\s*(\\d+)px`).exec(css)?.[1]);

// jsdom cannot measure text, so the column widths are held at their source. Each floor is an estimate of the widest
// content, worked out in the #201 review: the danger badge "Integrity check failed" (about 130 px of text, 18 px of
// badge padding and border, 16 px of cell padding), the Report and Verify buttons with their gap and the panel's
// padding, and the sortable "Passing" header (about 45 px at 12 px) with its cell padding.
describe("History column widths", () => {
  it("keeps Integrity wide enough for its longest badge, with or without a report open", () => {
    expect(width(".v-history .v-table th:nth-child(5)")).toBeGreaterThanOrEqual(164);
  });

  it("keeps Actions wide enough for Report and Verify side by side", () => {
    expect(width(".v-history .v-table th:nth-child(6)")).toBeGreaterThanOrEqual(135);
  });

  it("gives the open-report count column room for its header and a three-digit count", () => {
    expect(width(".v-history__body--open .v-table th:nth-child(3)")).toBeGreaterThanOrEqual(64);
  });
});
