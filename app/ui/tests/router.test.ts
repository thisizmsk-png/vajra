import { describe, expect, it } from "vitest";
import { parseHash, SCREENS, toHash } from "../src/router";

describe("hash router", () => {
  it("parses a screen and its params", () => {
    expect(parseHash("#/run?slug=a&id=b")).toEqual({ screen: "run", params: { slug: "a", id: "b" } });
  });
  it("sends empty, unknown and #kit-like hashes to home", () => {
    for (const h of ["", "#", "#/", "#/nope", "#/__proto__", "#/run/extra"]) expect(parseHash(h).screen).toBe("home");
  });
  it("round-trips every screen", () => {
    for (const s of SCREENS) expect(parseHash(toHash(s, { x: "a b&c" }))).toEqual({ screen: s, params: { x: "a b&c" } });
  });
});
