// The terminal output budget (FG-12 S12.5): one xterm write per frame, bounded bytes per frame, drop-to-tail when far
// behind, and a hidden pane that renders nothing. A manual scheduler and a manual xterm stand in for the browser.
import { describe, expect, it } from "vitest";
import { createOutputQueue } from "../src/terminal/outputQueue";

const bytes = (n: number, fill = 97) => new Uint8Array(n).fill(fill);
const text = (u: Uint8Array) => new TextDecoder().decode(u);

function rig(over: Partial<Parameters<typeof createOutputQueue>[0]> = {}) {
  const frames: (() => void)[] = [];
  const writes: Uint8Array[] = [];
  const dones: (() => void)[] = [];
  const acks: number[] = [];
  let scheduled = 0;
  const q = createOutputQueue({
    write: (d, done) => {
      writes.push(d);
      dones.push(done);
    },
    ack: (n) => void acks.push(n),
    schedule: (fn) => {
      scheduled++;
      frames.push(fn);
      return () => void frames.splice(frames.indexOf(fn) >>> 0, 1);
    },
    marker: (n) => new TextEncoder().encode(`[skipped ${Math.round(n / 1024)} KB]`),
    frameBytes: 1000,
    maxPending: 5000,
    hiddenCap: 2000,
    ...over,
  });
  const frame = () => frames.shift()?.();
  const parsed = () => dones.shift()?.();
  /** Run frames until the queue has nothing left to schedule. */
  const drain = () => {
    for (let guard = 0; frames.length && guard < 1000; guard++) {
      frame();
      while (dones.length) parsed();
    }
  };
  return { q, writes, acks, frame, parsed, drain, frames, get scheduled() { return scheduled; } };
}
const sum = (a: number[]) => a.reduce((x, y) => x + y, 0);

describe("coalescing", () => {
  it("GIVEN a thousand small messages in one frame WHEN the frame runs THEN xterm gets one write and one frame was scheduled", () => {
    const r = rig({ frameBytes: 100_000, maxPending: 1e9 });
    for (let i = 0; i < 1000; i++) r.q.push(bytes(10));
    expect(r.scheduled).toBe(1);
    r.frame();
    expect(r.writes).toHaveLength(1);
    expect(r.writes[0].length).toBe(10_000);
  });

  it("GIVEN more pending than a frame holds WHEN frames run THEN no write exceeds the frame budget and the order is kept", () => {
    const r = rig({ maxPending: 1e9 });
    r.q.push(Uint8Array.from({ length: 3500 }, (_, i) => i % 251));
    const all: number[] = [];
    for (let i = 0; i < 4; i++) {
      r.frame();
      r.parsed();
    }
    r.writes.forEach((w) => expect(w.length).toBeLessThanOrEqual(1000));
    r.writes.forEach((w) => all.push(...w));
    expect(all).toEqual(Array.from({ length: 3500 }, (_, i) => i % 251));
  });

  it("GIVEN a write xterm has not finished parsing WHEN more arrives THEN no second write starts until it is done", () => {
    const r = rig({ maxPending: 1e9 });
    r.q.push(bytes(2500));
    r.frame();
    r.q.push(bytes(100));
    expect(r.frames).toHaveLength(0); // nothing scheduled while one is in flight
    r.parsed();
    expect(r.frames).toHaveLength(1);
  });
});

describe("acknowledgements", () => {
  it("GIVEN written bytes WHEN xterm has parsed them THEN exactly those bytes are acknowledged, not before", () => {
    const r = rig({ maxPending: 1e9 });
    r.q.push(bytes(1500));
    r.frame();
    expect(r.acks).toEqual([]);
    r.parsed();
    expect(r.acks).toEqual([1000]);
    r.frame();
    r.parsed();
    expect(sum(r.acks)).toBe(1500);
  });
});

describe("far behind: drop to tail", () => {
  it("GIVEN a flood past the pending cap WHEN it is drained THEN the newest output is kept, a marker says what was skipped, and every byte is accounted for", () => {
    const r = rig({ frameBytes: 2000 });
    // 20 chunks of 1000, each tagged with its index in the first byte
    for (let i = 0; i < 20; i++) r.q.push(Uint8Array.from({ length: 1000 }, (_, k) => (k === 0 ? 65 + i : 46)));
    const kept = r.q.stats().pending;
    expect(kept).toBeLessThanOrEqual(5000);
    r.drain();
    const out = r.writes.map(text).join("");
    expect(out).toMatch(/^\[skipped \d+ KB\]/); // the marker comes first
    expect(out).toContain(String.fromCharCode(65 + 19)); // the newest chunk is there
    expect(out).not.toContain("A"); // the oldest is gone
    const acked = sum(r.acks);
    expect(acked).toBe(20_000); // dropped bytes were acknowledged when dropped, written ones when parsed
    expect(r.q.stats().dropped).toBeGreaterThan(0);
  });

  it("GIVEN a cut WHEN a line break is close THEN the kept output starts at a line start", () => {
    const r = rig({ maxPending: 1000, frameBytes: 5000 });
    const line = new TextEncoder().encode("0123456789".repeat(9) + "\n"); // 91 bytes per line
    for (let i = 0; i < 40; i++) r.q.push(line);
    r.drain();
    const body = text(r.writes[r.writes.length - 1]);
    expect(body.startsWith("0123456789")).toBe(true);
    expect(body.endsWith("\n")).toBe(true);
  });

  it("GIVEN no flood WHEN output is small THEN nothing is dropped and no marker is written", () => {
    const r = rig();
    r.q.push(bytes(500));
    r.frame();
    r.parsed();
    expect(r.writes.map(text).join("")).not.toContain("skipped");
    expect(r.q.stats().dropped).toBe(0);
  });
});

describe("hidden pane", () => {
  it("GIVEN a hidden pane WHEN output arrives THEN xterm is never written and no frame is scheduled", () => {
    const r = rig();
    r.q.setVisible(false);
    r.q.push(bytes(1500));
    expect(r.scheduled).toBe(0);
    expect(r.writes).toHaveLength(0);
  });

  it("GIVEN a hidden pane WHEN far more than its cap arrives THEN it holds the newest cap bytes and acknowledges the rest so the shell never stalls", () => {
    const r = rig();
    r.q.setVisible(false);
    for (let i = 0; i < 10; i++) r.q.push(bytes(1000, 65 + i));
    expect(r.q.stats().pending).toBeLessThanOrEqual(2000);
    expect(sum(r.acks)).toBeGreaterThanOrEqual(8000 - 100); // everything beyond what is held
    expect(sum(r.acks)).toBeLessThanOrEqual(10_000 - 2000 + 1000);
  });

  it("GIVEN a hidden pane that held output WHEN it is shown THEN the held tail is written, marker first, in frames", () => {
    const r = rig();
    r.q.setVisible(false);
    for (let i = 0; i < 10; i++) r.q.push(bytes(1000, 65 + i));
    r.q.setVisible(true);
    r.drain();
    const out = r.writes.map(text).join("");
    expect(out.startsWith("[skipped")).toBe(true);
    expect(out).toContain("J"); // the newest
    expect(out).not.toContain("A");
  });

  it("GIVEN a pane that was hidden mid-frame WHEN the frame fires THEN it does not write", () => {
    const r = rig();
    r.q.push(bytes(500));
    r.q.setVisible(false);
    r.frame();
    expect(r.writes).toHaveLength(0);
  });
});

describe("dispose", () => {
  it("GIVEN a disposed queue WHEN xterm finishes a write or more output arrives THEN nothing is written or acknowledged", () => {
    const r = rig({ maxPending: 1e9 });
    r.q.push(bytes(1500));
    r.frame();
    r.q.dispose();
    r.parsed();
    r.q.push(bytes(100));
    r.frame();
    expect(r.acks).toEqual([]);
    expect(r.writes).toHaveLength(1);
  });
});
