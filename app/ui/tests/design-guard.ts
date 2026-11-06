// K1 design guard (docs/desktop/DESIGN.md "Verification"). Scans screens and kit sources; token files are exempt.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

export type Violation = { file: string; line: number; rule: string; text: string };

const NAMED = "red|green|blue|white|black|gray|grey|orange|yellow|purple|pink|teal|cyan|magenta|navy|maroon|olive|lime|aqua|silver|gold|brown|crimson|coral|salmon|indigo|violet";
const RULES: Array<[string, RegExp]> = [
  ["raw hex color", /#[0-9a-fA-F]{3,8}\b(?![\w-])/],
  ["rgb()/hsl() value", /\b(?:rgba?|hsla?)\(/],
  ["letter-spacing", /letter-spacing|\btracking-[a-z0-9\[]/],
  // kebab-case in css, camelCase (`backgroundColor: "red"`) in a style object, with or without quotes around the value.
  ["named css color", new RegExp(`\\b(?:color|background(?:-color)?|border(?:-[a-z]+)?-color|fill|stroke|[a-z]+Color)\\s*:\\s*["']?(?:${NAMED})\\b`, "i")],
  ["gradient", /gradient\(/],
  ["banned webfont", /\b(?:Inter|Geist|Space Grotesk)\b/],
  ["ALL-CAPS transform", /text-transform\s*:\s*uppercase|\buppercase\b/],
];
// `#` inside a url fragment or a JSX/TS private field is not a color; the lookahead above keeps ids like #root out.
const EXEMPT_DIRS = ["theme", "node_modules", "dist", "tests"];

// A JSX style object may span lines, so it is matched on the whole text, not per line. Any colour property in it is
// a violation: colours come from tokens in a class, never inline.
const STYLE_BLOCK = /style=\{\{[^}]*\}\}/g;
const STYLE_COLOR = /\b(?:color|background|fill|stroke|[a-z]+Color)\s*:/g;

export function scanText(file: string, text: string): Violation[] {
  const out: Violation[] = [];
  const lineAt = (offset: number) => text.slice(0, offset).split("\n").length;
  for (const block of text.matchAll(STYLE_BLOCK)) {
    for (const prop of block[0].matchAll(STYLE_COLOR)) {
      const line = lineAt(block.index + prop.index);
      out.push({ file, line, rule: "inline style color", text: text.split("\n")[line - 1].trim() });
    }
  }
  text.split("\n").forEach((line, i) => {
    for (const [rule, re] of RULES) if (re.test(line)) out.push({ file, line: i + 1, rule, text: line.trim() });
  });
  return out;
}

export function scanTree(root: string): Violation[] {
  const out: Violation[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) {
        if (!EXEMPT_DIRS.includes(name)) walk(p);
      } else if (/\.(tsx?|css)$/.test(name) && name !== "vite.config.ts") {
        out.push(...scanText(relative(root, p), readFileSync(p, "utf8")));
      }
    }
  };
  walk(root);
  return out;
}
