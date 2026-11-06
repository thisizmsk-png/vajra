// K1 design guard (docs/desktop/DESIGN.md "Verification"). Scans screens and kit sources; token files are exempt.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

export type Violation = { file: string; line: number; rule: string; text: string };

const NAMED = "red|green|blue|white|black|gray|grey|orange|yellow|purple|pink|teal|cyan|magenta|navy|maroon|olive|lime|aqua|silver|gold|brown|crimson|coral|salmon|indigo|violet";
// A named colour as a whole word, not part of a token such as `--c-red` or `border-gray`.
const NAMED_WORD = `(?<![-\\w])(?:${NAMED})(?![-\\w])`;
const RULES: Array<[string, RegExp]> = [
  ["raw hex color", /#[0-9a-fA-F]{3,8}\b(?![\w-])/],
  // Every css colour function; `color-mix` is allowed on purpose, but only between tokens (see "color-mix literal").
  ["rgb()/hsl() value", /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(|\bcolor\(\s*(?:srgb(?:-linear)?|display-p3|a98-rgb|prophoto-rgb|rec2020|xyz(?:-d50|-d65)?)\b/i],
  ["color-mix literal", new RegExp(`color-mix\\([^;{}\\n]*${NAMED_WORD}`, "i")],
  ["letter-spacing", /letter-spacing|\btracking-[a-z0-9\[]/],
  // kebab-case in css, camelCase (`backgroundColor: "red"`) in a style object, with or without quotes around the value.
  ["named css color", new RegExp(`\\b(?:color|background(?:-color)?|border(?:-[a-z]+)?-color|fill|stroke|[a-z]+Color)\\s*:\\s*["']?(?:${NAMED})\\b`, "i")],
  // Shorthands that carry a colour among other values (`border: 1px solid red`), in css and in a style object.
  ["named colour in shorthand", new RegExp(`\\b(?:border(?:-[a-z]+)?|outline|box-shadow|boxShadow|text-decoration|textDecoration|column-rule|border[A-Z]\\w*|outline[A-Z]\\w*)\\s*:[^;{}\\n]*${NAMED_WORD}`, "i")],
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
