// Provider tiles: a shipped mark is exactly the file in SOURCES.md, the tile keeps its size, the name is announced only where no text
// name sits beside it, and a provider without a cleared mark keeps its monogram.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MARKS } from "../src/assets/providers/marks";
import { ProviderIcon } from "../src/screens/ProviderIcon";

const DIR = join(__dirname, "../src/assets/providers");
const sources = readFileSync(join(DIR, "SOURCES.md"), "utf8");
const rows = [...sources.matchAll(/^\| `([^`]+\.svg)` \|.*?\| `([0-9a-f]{64})` \|/gm)].map((m) => ({ file: m[1], sha: m[2] }));

describe("provider icon files", () => {
  it("every svg in the folder has a row in SOURCES.md, and its sha256 matches", () => {
    const files = readdirSync(DIR).filter((f) => f.endsWith(".svg")).sort();
    expect(rows.map((r) => r.file).sort()).toEqual(files);
    for (const r of rows) expect(createHash("sha256").update(readFileSync(join(DIR, r.file))).digest("hex"), r.file).toBe(r.sha);
  });
  it("a shipped mark has a source URL, a download date and its terms quoted", () => {
    for (const f of ["openrouter-ink.svg", "openrouter-cloud.svg"]) {
      const row = sources.split("\n").find((l) => l.startsWith(`| \`${f}\``))!;
      expect(row).toMatch(/https:\/\/openrouter\.ai\/brand\/logos\//);
      expect(row).toMatch(/\d{4}-\d{2}-\d{2}/);
      expect(row).toMatch(/don't stretch, recolor, or remix/);
    }
  });
  it("only providers whose terms are cleared have a mark", () => {
    expect(Object.keys(MARKS)).toEqual(["openrouter"]);
  });
  it("the app code never reaches the network for an icon", () => {
    const code = readFileSync(join(__dirname, "../src/screens/ProviderIcon.tsx"), "utf8") + readFileSync(join(DIR, "marks.ts"), "utf8");
    expect(code).not.toMatch(/https?:\/\//);
  });
});

describe("ProviderIcon", () => {
  const tile = (el: JSX.Element) => render(el).container.querySelector(".v-pt") as HTMLElement;
  it("a cleared provider shows its two files (one per theme), unaltered", () => {
    const t = tile(<ProviderIcon family="openrouter" />);
    const imgs = [...t.querySelectorAll("img")];
    expect(imgs.map((i) => i.getAttribute("src"))).toEqual([MARKS.openrouter.light, MARKS.openrouter.dark]);
    expect(imgs.every((i) => i.getAttribute("alt") === "")).toBe(true);
    expect(t.textContent).toBe("");
  });
  it("a provider without a cleared mark keeps its monogram; an unknown one gets the fallback icon", () => {
    expect(tile(<ProviderIcon family="claude" />).textContent).toBe("CC");
    expect(tile(<ProviderIcon family="deepseek" />).textContent).toBe("DS");
    const u = tile(<ProviderIcon family="mystery" />);
    expect(u.querySelector("svg")).not.toBeNull();
    expect(u.getAttribute("data-family")).toBe("unknown");
  });
  it("a gateway model keeps its own tile with the gateway mark as the badge", () => {
    const t = tile(<ProviderIcon family="deepseek" via="openrouter" />);
    expect(t.textContent).toBe("DS");
    expect(t.querySelectorAll(".v-pt__via img")).toHaveLength(2);
  });
  it("with a name beside it the tile is hidden from assistive tech; alone it is an image named for the provider", () => {
    const beside = tile(<ProviderIcon family="claude" />);
    expect(beside.getAttribute("aria-hidden")).toBe("true");
    expect(beside.getAttribute("role")).toBeNull();
    const alone = tile(<ProviderIcon family="deepseek" via="openrouter" named />);
    expect(alone.getAttribute("role")).toBe("img");
    expect(alone.getAttribute("aria-label")).toBe("DeepSeek · OpenRouter");
    expect(alone.getAttribute("aria-hidden")).toBeNull();
    expect(tile(<ProviderIcon family="claude" named />).getAttribute("aria-label")).toBe("Claude Code");
  });
});
