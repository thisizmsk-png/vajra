import { describe, expect, it } from "vitest";
import { ago } from "../src/screens/when";

const now = Date.parse("2026-11-07T12:00:00");
describe("ago", () => {
  it("counts whole units that have passed, with numbers", () => {
    expect(ago("2026-11-01 00:00", now)).toMatch(/6 days ago/);
    expect(ago("2026-10-31 11:00", now)).toMatch(/1 week ago/);
    expect(ago("2026-11-07 09:30", now)).toMatch(/2 hours ago/);
  });
  it("shows a value it cannot read as it is, and nothing for none", () => {
    expect(ago("soon", now)).toBe("soon");
    expect(ago(undefined, now)).toBe("");
  });
});
