// @vitest-environment node
// The marks as the build ships them: same-origin files, never data: URIs (the app's CSP, default-src 'self', blocks data: images).
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { build } from "vite";
import { describe, expect, it } from "vitest";

describe("built marks", () => {
  it("ship as same-origin files, never data: URIs (the CSP blocks data: images)", async () => {
    const out = await build({
      configFile: join(__dirname, "../vite.config.ts"), root: join(__dirname, ".."), logLevel: "silent",
      build: { write: false, outDir: "dist-test" },
    });
    const items = (Array.isArray(out) ? out : [out]).flatMap((o) => ("output" in o ? o.output : []));
    const code = items.filter((i) => i.type === "chunk").map((i) => (i as { code: string }).code).join("\n");
    const files = items.filter((i) => i.type === "asset").map((i) => i.fileName);
    expect(code).not.toMatch(/data:image/);
    const images = files.filter((f) => /\.(svg|png|ico)$/.test(f));
    const onDisk = (d: string): string[] => readdirSync(d).flatMap((n) => (statSync(join(d, n)).isDirectory() ? onDisk(join(d, n)) : [n]));
    const shipped = onDisk(join(__dirname, "../src/assets/providers")).filter((n) => /\.(svg|png|ico)$/.test(n) && n !== "fallback.svg"); // fallback.svg is inlined as markup
    expect(images).toHaveLength(shipped.length); // every mark file is its own same-origin asset
  }, 30000);
});
