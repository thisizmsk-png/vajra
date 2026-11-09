import { describe, expect, it } from "vitest";
import { parseHash, SCREENS, toHash } from "../src/router";

describe("hash router", () => {
  it("parses a project-scoped view and its params", () => {
    expect(parseHash("#/p/p_0a1b2c3d/board?x=1")).toEqual({ screen: "board", params: { x: "1" }, project: "p_0a1b2c3d" });
    expect(parseHash("#/p/p_0a1b2c3d/review?slug=a&id=b")).toEqual({ screen: "review", params: { slug: "a", id: "b" }, project: "p_0a1b2c3d" });
  });

  it("keeps every old link working: old screen names are aliases of the new views, with no project", () => {
    expect(parseHash("#/run?slug=a&id=b")).toEqual({ screen: "review", params: { slug: "a", id: "b" }, project: null });
    expect(parseHash("#/home").screen).toBe("orch");
    expect(parseHash("#/chat").screen).toBe("orch");
    expect(parseHash("#/inbox").screen).toBe("inbox");
    expect(parseHash("#/crew?slug=camp&id=r1")).toEqual({ screen: "crew", params: { slug: "camp", id: "r1" }, project: null });
    expect(parseHash("#/history").screen).toBe("history");
    expect(parseHash("#/settings?section=mobile")).toEqual({ screen: "settings", params: { section: "mobile" }, project: null });
  });

  it("sends empty, unknown and #kit-like hashes to the orchestrator view", () => {
    for (const h of ["", "#", "#/", "#/nope", "#/__proto__", "#/run/extra", "#/p/", "#/p/p_1/nope", "#/p/p_1/board/extra"]) {
      expect(parseHash(h).screen).toBe("orch");
    }
  });

  it("drops a project id that is not a plain token, rather than trusting it", () => {
    expect(parseHash("#/p/..%2Fetc/board").project).toBeNull();
    expect(parseHash("#/p/a b/board").project).toBeNull();
  });

  it("round-trips every view, with and without a project", () => {
    for (const s of SCREENS) {
      expect(parseHash(toHash(s, { x: "a b&c" }))).toEqual({ screen: s, params: { x: "a b&c" }, project: null });
      expect(parseHash(toHash(s, { x: "1" }, "p_00ff"))).toEqual({ screen: s, params: { x: "1" }, project: "p_00ff" });
    }
  });

  it("writes old names as their new view", () => {
    expect(toHash("run", { slug: "a" }, "p_1")).toBe("#/p/p_1/review?slug=a");
    expect(toHash("home")).toBe("#/orch");
  });
});
