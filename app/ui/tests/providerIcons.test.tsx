// Provider tiles: a shipped mark is exactly the file in SOURCES.md, the tile keeps its size, the name is announced only where no text
// name sits beside it, and a provider without a cleared mark keeps its monogram.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fireEvent, render } from "@testing-library/react";
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
    for (const f of ["OAI_OpenAI-Blossom_Black.svg", "OAI_OpenAI-Blossom_White.svg", "Grok_Logomark_Dark.svg", "Grok_Logomark_Light.svg"]) {
      const row = sources.split("\n").find((l) => l.startsWith(`| \`${f}\``))!;
      expect(row, f).toMatch(/https:\/\/(openai\.com\/brand|x\.ai\/legal\/brand-guidelines)|same (kit|page)/);
      expect(row, f).toMatch(/2026-11-10/);
      expect(row, f).toMatch(/accepted the usage terms and the clearance risk 2026-11-10/);
    }
  });
  it("each mark records how much of its canvas it fills, so the tile can size every mark alike", () => {
    for (const [id, m] of Object.entries(MARKS)) expect(m.fill, id).toBeGreaterThan(0.3);
    for (const [id, m] of Object.entries(MARKS)) expect(m.fill, id).toBeLessThanOrEqual(1);
    expect(MARKS.codex.fill).toBeLessThan(MARKS.grok.fill); // the Blossom file keeps a wide margin around the flower
  });
  it("only providers whose terms are cleared have a mark", () => {
    expect(Object.keys(MARKS).sort()).toEqual(["codex", "grok", "openrouter"]); // Claude Code and DeepSeek keep their monograms
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
    expect(imgs.every((i) => i.style.getPropertyValue("--fill") === String(MARKS.openrouter.fill))).toBe(true);
    expect(t.textContent).toBe("");
  });
  it("an image that fails to load falls back to the monogram, never an empty tile", () => {
    const t = tile(<ProviderIcon family="openrouter" />);
    fireEvent.error(t.querySelector("img")!);
    expect(t.querySelector("img")).toBeNull();
    expect(t.textContent).toBe("OR");
    const b = tile(<ProviderIcon family="deepseek" via="openrouter" />);
    fireEvent.error(b.querySelector(".v-pt__via img")!);
    expect(b.querySelector(".v-pt__via")?.textContent).toBe("OR");
  });
  it("a provider without a cleared mark keeps its monogram; an unknown one gets the fallback icon", () => {
    expect(tile(<ProviderIcon family="claude" />).textContent).toBe("CC");
    expect(tile(<ProviderIcon family="deepseek" />).textContent).toBe("DS");
    expect(tile(<ProviderIcon family="codex" />).querySelectorAll("img")).toHaveLength(2);
    expect(tile(<ProviderIcon family="grok" />).querySelectorAll("img")).toHaveLength(2);
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
