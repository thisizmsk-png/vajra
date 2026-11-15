// "The composer never waits": typing re-renders only the composer, a stream of new lines never re-renders it, a refused
// ask hands the text back, and one expensive read has one owner however many callers ask at once.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { CurrentProject } from "../src/projects/current";
import type { Current } from "../src/projects/current";
import { DirectorGeneral } from "../src/screens/orch/DirectorGeneral";
import { forgetRead, ownedRead, readCounts } from "../src/reads";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const STREAM = "orchestrator-p_X-g3";
const status = { available: true, busy: false, name: "Director-General", state: "idle", reason: "waiting", project_id: "p_X", generation: 3, stream: STREAM };
const win = (n: number) => ({ lines: Array.from({ length: n }, (_, i) => ({ seq: i + 1, text: `lead\tline ${i}` })), head_seq: n, first_seq: 1, at_start: true, at_end: true, gap: false });
const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
const cur: Current = { project, campaigns: [], focus: "c1", runId: null, dgRun: "20261106-2140", proposals: [] };
const composer = () => screen.getByRole("textbox", { name: S.dg.ask }) as HTMLTextAreaElement;

afterEach(() => { cleanup(); forgetRead(); });

describe("composer isolation", () => {
  it("keeps the draft and the cursor while the stream grows, and typing never calls the core", async () => {
    let lines = 5;
    const f = fakeIpc({ lead_status: status, stream_window: () => win(lines) });
    setTransport(f.transport);
    render(<CurrentProject.Provider value={cur}><DirectorGeneral /></CurrentProject.Provider>);
    await screen.findByRole("log", { name: S.dg.transcript });
    const box = composer();
    fireEvent.change(box, { target: { value: "half a question" } });
    box.setSelectionRange(4, 4);
    const before = f.calls.length;
    for (let i = 0; i < 60; i++) fireEvent.change(box, { target: { value: `half a question ${i}` } });
    expect(f.calls.length).toBe(before);
    lines = 40;
    await act(async () => { await new Promise((r) => setTimeout(r, 0)); });
    expect(composer()).toBe(box);
    expect(box.value).toBe("half a question 59");
  });

  it("shows my question at once, clears the box, and puts the text back when the core refuses", async () => {
    let release: (v: unknown) => void = () => {};
    const f = fakeIpc({ lead_status: status, stream_window: () => win(2), ask_lead: () => new Promise((r) => { release = r; }) });
    setTransport(f.transport);
    render(<CurrentProject.Provider value={cur}><DirectorGeneral /></CurrentProject.Provider>);
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.change(composer(), { target: { value: "Is it done?" } });
    fireEvent.click(screen.getByRole("button", { name: S.dg.send }));
    // Before any answer: the question is on screen and the box is empty.
    expect(await screen.findByText("Is it done?")).toBeTruthy();
    expect(composer().value).toBe("");
    await act(async () => { release(Promise.reject({ error: "lead_busy", detail: "busy" })); await new Promise((r) => setTimeout(r, 0)); });
    await waitFor(() => expect(composer().value).toBe("Is it done?"));
    expect(screen.queryByText("Is it done?", { selector: ".v-turn__t" })).toBeNull();
  });
});

describe("one owner per read", () => {
  it("answers concurrent callers with one call, and a failed read is not kept", async () => {
    const read = vi.fn(async () => "x");
    const [a, b, c] = await Promise.all([ownedRead("k", read), ownedRead("k", read), ownedRead("k", read)]);
    expect([a, b, c]).toEqual(["x", "x", "x"]);
    expect(read).toHaveBeenCalledTimes(1);
    expect(readCounts().hits).toBeGreaterThanOrEqual(2);
    const bad = vi.fn().mockRejectedValueOnce(new Error("no")).mockResolvedValueOnce("ok");
    await expect(ownedRead("f", bad)).rejects.toThrow("no");
    await expect(ownedRead("f", bad)).resolves.toBe("ok");
  });

  it("serves a kept value inside the freshness window only", async () => {
    vi.useFakeTimers();
    try {
      const read = vi.fn(async () => "v");
      await ownedRead("w", read, 5000);
      await ownedRead("w", read, 5000);
      expect(read).toHaveBeenCalledTimes(1);
      vi.advanceTimersByTime(5001);
      await ownedRead("w", read, 5000);
      expect(read).toHaveBeenCalledTimes(2);
    } finally { vi.useRealTimers(); }
  });
});
