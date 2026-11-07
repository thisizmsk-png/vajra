import { afterEach, describe, expect, it } from "vitest";
import { api, errorText, isIpcError, onChanged, setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";

afterEach(() => setTransport());

describe("typed ipc", () => {
  it("sends camelCase arguments under the command's name", async () => {
    const f = fakeIpc({ tail: { lines: [], gap: false, head_seq: 0, spill_start: null, spill_end: null }, take_over: {} });
    setTransport(f.transport);
    await api.tail("s1", 7);
    await api.takeOver("s1", true);
    expect(f.calls).toEqual([
      { cmd: "tail", args: { stream: "s1", afterSeq: 7, maxBytes: 65536 } },
      { cmd: "take_over", args: { stream: "s1", on: true } },
    ]);
  });

  it("maps the three Lead commands to snake_case commands with camelCase args", async () => {
    const f = fakeIpc({ ask_lead: {}, cancel_ask: {}, lead_status: { available: true, busy: false } });
    setTransport(f.transport);
    await api.askLead("20261106-0911", "what is bhima doing?");
    await api.cancelAsk("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    await api.leadStatus("20261106-0911");
    expect(f.calls).toEqual([
      { cmd: "ask_lead", args: { runId: "20261106-0911", text: "what is bhima doing?" } },
      { cmd: "cancel_ask", args: { askId: "01ARZ3NDEKTSV4RRFFQ69G5FAV" } },
      { cmd: "lead_status", args: { runId: "20261106-0911" } },
    ]);
  });

  it("rejects with the core's {error, detail} and errorText reads it", async () => {
    setTransport(fakeIpc({ preflight: () => { throw { error: "no_claude", detail: "not on PATH" }; } }).transport);
    const e = await api.preflight().catch((x) => x);
    expect(isIpcError(e)).toBe(true);
    expect(errorText(e)).toBe("no_claude: not on PATH");
  });

  it("never shows a stack or an unknown object", () => {
    expect(errorText(new Error("boom\n at x"))).toBe("unexpected error");
    expect(errorText(null)).toBe("unexpected error");
    expect(errorText({ error: "gone" })).toBe("gone");
  });

  it("onChanged subscribes to the changed event and unsubscribes", () => {
    const f = fakeIpc();
    setTransport(f.transport);
    const seen: unknown[] = [];
    const off = onChanged((p) => seen.push(p));
    f.emit("changed", ["a"]);
    off();
    f.emit("changed", ["b"]);
    expect(seen).toEqual([["a"]]);
    expect(f.listenerCount("changed")).toBe(0);
  });
});
