import { describe, expect, it } from "vitest";
import { S } from "../strings";

/** Every visible string: plain ones as they are, and each string-building function called with sample arguments. */
function visible(node: unknown, path: string, out: Array<[string, string]>) {
  if (typeof node === "string") out.push([path, node]);
  else if (typeof node === "function") {
    try {
      const made = (node as (...a: unknown[]) => unknown)(2, 3, 4, 5);
      if (typeof made === "string") out.push([path, made]);
    } catch { /* a builder that needs richer arguments is covered by its own screen's tests */ }
  } else if (node && typeof node === "object") {
    for (const [k, v] of Object.entries(node)) visible(v, path ? `${path}.${k}` : k, out);
  }
}

describe("wording", () => {
  it("says run and workers, never campaign or crew, in anything a person reads", () => {
    const all: Array<[string, string]> = [];
    visible(S, "", all);
    expect(all.length).toBeGreaterThan(300);
    expect(all.filter(([, text]) => /\b(campaigns?|crews?)\b/i.test(text)).map(([path, text]) => `${path}: ${text}`)).toEqual([]);
  });
});
