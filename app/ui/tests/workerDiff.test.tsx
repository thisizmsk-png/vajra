// A worker's change count (ADR-16 16.10): `+N −M` when exact, `≥ +N lines` with a plain-words label when some new files were not counted.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { DiffCount } from "../src/screens/ProviderLine";
import { diffLabel, diffText, parseWorkerDiff } from "../src/workerProvider";

afterEach(cleanup);
const exact = { files_total: 3, files: ["a.txt", "b.txt", "c.txt"], added: 12, removed: 4, partial: false, uncounted: 0 };
const partial = { files_total: 6000, files: ["n0.txt"], added: 5000, removed: 0, partial: true, uncounted: 1000 };

describe("the worker diff", () => {
  it("parses the core's shape and drops anything that is not one", () => {
    expect(parseWorkerDiff(exact)).toEqual(exact);
    for (const bad of [null, "x", [], {}, { ...exact, added: -1 }, { ...exact, added: 1.5 }, { ...exact, partial: "no" }, { ...exact, files_total: "3" }]) expect(parseWorkerDiff(bad)).toBeNull();
    expect(parseWorkerDiff({ ...exact, uncounted: undefined })?.uncounted).toBe(0);
  });

  it("lists at most 200 files and drops a name with a control or invisible character", () => {
    const many = Array.from({ length: 300 }, (_, i) => `f${i}.txt`);
    expect(parseWorkerDiff({ ...exact, files: many })?.files).toHaveLength(200);
    expect(parseWorkerDiff({ ...exact, files: ["ok.txt", "bad\u0007.txt", "hid​den.txt", 7] })?.files).toEqual(["ok.txt"]);
  });

  it("says +N −M for an exact count", () => {
    const d = parseWorkerDiff(exact)!;
    expect(diffText(d)).toBe("+12 −4");
    expect(diffLabel(d)).toBe("12 lines added, 4 removed, in 3 files.");
    render(<DiffCount d={d} />);
    expect(screen.getByLabelText("12 lines added, 4 removed, in 3 files.").textContent).toBe("+12 −4");
  });

  it("says ≥ +N lines, with the label, when some new files were not counted", () => {
    const d = parseWorkerDiff(partial)!;
    expect(diffText(d)).toBe("≥ +5000 lines");
    expect(diffLabel(d)).toBe("At least 5000 lines added. 1000 new files were not counted (too many or too large).");
    render(<DiffCount d={d} />);
    const el = screen.getByLabelText(diffLabel(d));
    expect(el.textContent).toBe("≥ +5000 lines");
    expect(diffLabel({ ...d, uncounted: 1 })).toContain("1 new file was not counted");
  });
});
