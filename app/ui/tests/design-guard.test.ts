import { describe, expect, it } from "vitest";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { scanText, scanTree } from "./design-guard";

const UI = join(dirname(fileURLToPath(import.meta.url)), "..");

describe("design guard", () => {
  it("catches each banned pattern (so a pass means something)", () => {
    const bad: Record<string, string> = {
      "raw hex color": "a { color: #d97757; }",
      "rgb()/hsl() value": "a { color: rgb(1 2 3); }",
      "letter-spacing": "a { letter-spacing: 0.1em; }",
      "inline style color": '<b style={{ color: "x" }} />',
      "named css color": "a { background: red; }",
      gradient: "a { background: linear-gradient(red, blue); }",
      "banned webfont": "a { font-family: Inter; }",
      "ALL-CAPS transform": "a { text-transform: uppercase; }",
    };
    for (const [rule, text] of Object.entries(bad)) {
      expect(scanText("x", text).map((v) => v.rule), rule).toContain(rule);
    }
  });

  it("catches multiline style objects and camelCase colour properties", () => {
    const multi = ["<div style={{", '  backgroundColor: "red"', "}} />"].join("\n");
    const vs = scanText("x", multi);
    expect(vs.map((v) => v.rule)).toEqual(expect.arrayContaining(["inline style color", "named css color"]));
    expect(vs.find((v) => v.rule === "inline style color")?.line).toBe(2);
    for (const prop of ["borderColor", "color", "fill", "stroke", "outlineColor"]) {
      const text = `<i style={{\n  padding: 4,\n  ${prop}: "x",\n}} />`;
      expect(scanText("x", text).map((v) => v.rule), prop).toContain("inline style color");
    }
    expect(scanText("x", '<i style={{ padding: 4, display: "flex" }} />')).toEqual([]);
  });

  it("does not flag tokens, ids or ordinary css", () => {
    for (const ok of ["a { color: var(--c-text); }", "#root { margin: 0 }", "color-mix(in srgb, var(--c-a) 60%, transparent)", "const x = `#${id}`"]) {
      expect(scanText("x", ok), ok).toEqual([]);
    }
  });

  it("baseline: the kit and every screen have 0 violations", () => {
    expect(scanTree(UI)).toEqual([]);
  });
});
