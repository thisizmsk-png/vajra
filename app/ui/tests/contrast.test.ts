import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(HERE, "../theme/tokens.css"), "utf8");
const design = readFileSync(join(HERE, "../../../docs/desktop/DESIGN.md"), "utf8");

function block(selectorStart: string): Record<string, string> {
  const i = css.indexOf(selectorStart);
  const body = css.slice(css.indexOf("{", i) + 1, css.indexOf("}", i));
  const vars: Record<string, string> = {};
  for (const m of body.matchAll(/--([a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) vars[m[1]] = m[2].toLowerCase();
  return vars;
}
const THEMES = { dark: block(":root,"), light: block('[data-theme="light"] {') };

const lum = (hex: string) => {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

const BGS = ["background", "surface", "surface-elevated", "accent"];
const TEXT_ROWS = ["text", "text-secondary", "text-muted", "primary", "success", "warning", "danger", "info", "focus-ring"];

describe.each(["dark", "light"] as const)("%s theme contrast", (theme) => {
  const t = THEMES[theme];
  it("defines every role", () => {
    for (const k of [...BGS, ...TEXT_ROWS, "border", "border-strong", "text-on-primary"]) expect(t[`c-${k}`], k).toBeTruthy();
  });
  it("text and status colors reach 4.5:1 on all four surfaces", () => {
    for (const fg of TEXT_ROWS.filter((r) => r !== "focus-ring")) {
      for (const bg of BGS) expect(ratio(t[`c-${fg}`], t[`c-${bg}`]), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
  });
  it("border-strong and focus-ring reach 3:1", () => {
    for (const fg of ["border-strong", "focus-ring"]) {
      for (const bg of BGS) expect(ratio(t[`c-${fg}`], t[`c-${bg}`]), `${fg} on ${bg}`).toBeGreaterThanOrEqual(3);
    }
  });
  it("the label on a primary button reaches 4.5:1", () => {
    expect(ratio(t["c-text-on-primary"], t["c-primary"])).toBeGreaterThanOrEqual(4.5);
  });
  it("the ink primary: its label reaches 4.5:1 and its fill 3:1 against the surfaces it sits on", () => {
    expect(ratio(t["c-text-on-action"], t["c-action"])).toBeGreaterThanOrEqual(4.5);
    for (const bg of [...BGS, "panel"]) expect(ratio(t["c-action"], t[`c-${bg}`]), `action on ${bg}`).toBeGreaterThanOrEqual(3);
  });
  it("text, status colors and border-strong hold on a panel", () => {
    for (const fg of TEXT_ROWS.filter((r) => r !== "focus-ring")) expect(ratio(t[`c-${fg}`], t["c-panel"]), `${fg} on panel`).toBeGreaterThanOrEqual(4.5);
    for (const fg of ["border-strong", "focus-ring"]) expect(ratio(t[`c-${fg}`], t["c-panel"]), `${fg} on panel`).toBeGreaterThanOrEqual(3);
  });
  it("all 16 ANSI colors reach 4.5:1 on the background", () => {
    const ansi = Object.entries(t).filter(([k]) => k.startsWith("ansi-"));
    expect(ansi).toHaveLength(16);
    for (const [k, v] of ansi) expect(ratio(v, t["c-background"]), k).toBeGreaterThanOrEqual(4.5);
  });
});

describe("a disabled ghost button stays legible", () => {
  // Disabled text is exempt from WCAG 1.4.3, so this is our own floor: 3:1, the level asked of large text and icons.
  const base = readFileSync(join(HERE, "../components/ui/base/base.css"), "utf8");
  const opacity = Number(/\.v-button--ghost\[aria-disabled="true"\]\s*\{\s*opacity:\s*([\d.]+)/.exec(base)?.[1]);
  const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  const hex = (c: number[]) => `#${c.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
  /** The colour a label shows when its button is drawn at `o` opacity over `bg`. */
  const faded = (fg: string, bg: string, o: number) => hex(rgb(fg).map((v, i) => v * o + rgb(bg)[i] * (1 - o)));

  it("has its own opacity, and it dims the label", () => {
    expect(opacity).toBeGreaterThan(0);
    expect(opacity).toBeLessThan(1);
  });
  it.each(["dark", "light"] as const)("%s: the label reaches 3:1 on every surface and stays under an enabled muted label", (theme) => {
    const t = THEMES[theme];
    for (const bg of ["background", "surface", "surface-elevated", "panel"]) {
      const got = ratio(faded(t["c-text-secondary"], t[`c-${bg}`], opacity), t[`c-${bg}`]);
      expect(got, `disabled ghost on ${bg}`).toBeGreaterThanOrEqual(3);
      expect(got, `disabled ghost on ${bg}`).toBeLessThan(ratio(t["c-text-muted"], t[`c-${bg}`]));
    }
  });
});

describe("DESIGN.md contrast tables are reproduced", () => {
  // Rows look like "| text #e6edf3 | 16.02 | 14.64 | 13.70 | 12.88 |" under a "Dark fg on:" or "Light fg on:" header.
  for (const [theme, header] of [["dark", "| Dark fg on:"], ["light", "| Light fg on:"]] as const) {
    it(`${theme} table matches the computed ratios to 0.01`, () => {
      const lines = design.slice(design.indexOf(header)).split("\n").slice(2);
      let checked = 0;
      for (const line of lines) {
        if (!line.startsWith("|")) break;
        const cells = line.split("|").map((c: string) => c.trim()).filter(Boolean);
        const names = cells[0].replace(/#[0-9a-f]{6}/i, "").trim().split(",").map((n: string) => n.trim());
        const nums = cells.slice(1).map(Number);
        for (const name of names) {
          nums.forEach((want: number, i: number) => {
            const got = ratio(THEMES[theme][`c-${name}`], THEMES[theme][`c-${BGS[i]}`]);
            expect(Math.abs(got - want), `${theme} ${name} on ${BGS[i]}: doc ${want}, computed ${got.toFixed(2)}`).toBeLessThanOrEqual(0.011);
            checked++;
          });
        }
      }
      expect(checked).toBeGreaterThanOrEqual(36);
    });
  }
});
