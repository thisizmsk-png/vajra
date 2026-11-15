import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// Tauri v2 sends commands over the custom protocols `ipc://localhost` (macOS, Linux) and `http://ipc.localhost`
// (Windows). With only `default-src 'self'`, the webview refuses those requests (connect-src falls back to
// default-src) and Tauri retries over its postMessage fallback on every call. Allowing exactly those two sources,
// and nothing else, keeps IPC on the fast path without widening the policy.
const directives = (csp: string) => Object.fromEntries(csp.split(";").map((d) => d.trim().split(/\s+/)).filter((d) => d[0]).map(([name, ...src]) => [name, src]));

describe("the IPC part of the CSP", () => {
  const csp: string = JSON.parse(readFileSync(join(process.cwd(), "../core/tauri.conf.json"), "utf8")).app.security.csp;
  const d = directives(csp);

  it("lets the webview reach Tauri's IPC protocols, and only those besides itself", () => {
    expect(d["connect-src"]).toEqual(["'self'", "ipc:", "http://ipc.localhost"]);
  });
  it("leaves everything else as it was: default-src 'self', no script-src of our own, nothing unsafe or wildcard", () => {
    expect(d["default-src"]).toEqual(["'self'"]);
    expect(d["script-src"]).toBeUndefined(); // Tauri adds its own hash at build time; the config must not widen it
    expect(csp).not.toMatch(/\*|https?:\/\/(?!ipc\.localhost)|data:|blob:|'unsafe-eval'/);
    expect(d["connect-src"].join(" ")).not.toMatch(/unsafe/);
  });
});
