import { describe, expect, it } from "vitest";
import { runStatusKey, taskStatusKey, timelineStatusKey } from "../src/statusMap";
import { S } from "../strings";

describe("one status map", () => {
  it("maps every run, task and timeline state the core sends onto the status vocabulary", () => {
    const keys = Object.keys(S.status);
    for (const s of ["active", "done", "stopped", "failed", "abandoned"]) expect(keys).toContain(runStatusKey(s));
    for (const s of ["pending", "ready", "claimed", "in_progress", "question", "blocked", "closed", "done", "failed"]) expect(keys).toContain(taskStatusKey(s));
    for (const s of ["active", "done", "need_input", "rejected"]) expect(keys).toContain(timelineStatusKey(s));
  });
  it("reads one state the same way wherever it appears", () => {
    expect(runStatusKey("active")).toBe(taskStatusKey("in_progress"));
    expect(timelineStatusKey("active")).toBe("in_progress");
    expect(taskStatusKey("closed")).toBe(taskStatusKey("done"));
    expect(runStatusKey("done")).toBe("done");
  });
  it("does not invent a word for a state it does not know", () => {
    expect(taskStatusKey("odd_new_state")).toBeNull();
    expect(runStatusKey("odd_new_state")).toBe("unknown");
  });
});
