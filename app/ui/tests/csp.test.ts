import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// xterm.js creates <style> elements at runtime, which needs `style-src 'unsafe-inline'`. Tauri adds a nonce or hashes to
// `style-src` at build time when the HTML carries its style nonce token or inline <style> elements; browsers then IGNORE
// 'unsafe-inline' and xterm's styles break. (tauri 2.11 `manager::set_csp` / `replace_csp_nonce`: it only does so when
// the asset has a nonce token or the build found style hashes.) So the entry HTML must keep both out.
describe("the style CSP the terminal depends on", () => {
  const html = readFileSync(join(process.cwd(), "index.html"), "utf8");
  const conf = JSON.parse(readFileSync(join(process.cwd(), "../core/tauri.conf.json"), "utf8"));

  it("allows inline styles for the whole app, styles only", () => {
    const csp: string = conf.app.security.csp;
    expect(csp).toMatch(/style-src [^;]*'unsafe-inline'/);
    expect(csp).not.toMatch(/script-src[^;]*'unsafe-inline'/);
  });
  it("keeps Tauri from adding a nonce or hash that would switch 'unsafe-inline' off", () => {
    expect(html).not.toMatch(/<style[\s>]/i);
    expect(html).not.toContain("__TAURI_STYLE_NONCE__");
  });
});
