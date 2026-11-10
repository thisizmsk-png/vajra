// @vitest-environment node
// The marks as the build ships them: same-origin files, never data: URIs (the app's CSP, default-src 'self', blocks data: images).
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
    expect(files.filter((f) => f.endsWith(".svg"))).toHaveLength(7); // a file per theme for each of three providers, plus the one Claude spark
    expect(files.filter((f) => f.endsWith(".ico"))).toHaveLength(1); // the DeepSeek icon, shipped as the file it is
  }, 30000);
});
