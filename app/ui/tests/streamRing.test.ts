import { describe, expect, it } from "vitest";
import { EARLIER_PAGE, MAX_ROWS, RING, appendTail, emptyRing, prependEarlier } from "../src/screens/crew/streamRing";

const lines = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => ({ seq: from + i, text: `l${from + i}` }));
const tail = (ls: { seq: number; text: string }[], gap = false) => ({ lines: ls, gap, head_seq: ls.at(-1)?.seq ?? 0, spill_start: null, spill_end: null });
const texts = (r: { rows: { text: string }[] }) => r.rows.map((x) => x.text);

describe("stream ring (S2b)", () => {
  it("appends only lines newer than the last seen, so a re-sent tail adds nothing", () => {
    let r = appendTail(emptyRing, tail(lines(1, 3)), RING);
    r = appendTail(r, tail(lines(2, 5)), RING);
    expect(texts(r)).toEqual(["l1", "l2", "l3", "l4", "l5"]);
    expect(r.last).toBe(5);
  });

  it("never holds more than the cap, however much output arrives, and keeps the newest", () => {
    let r = emptyRing;
    for (let i = 0; i < 20; i++) r = appendTail(r, tail(lines(i * 1000 + 1, (i + 1) * 1000)), RING);
    expect(r.rows.length).toBeLessThanOrEqual(RING);
    expect(r.rows.at(-1)?.text).toBe("l20000");
    expect(r.rows[0].text).toBe(`l${20000 - RING + 1}`);
  });

  it("marks skipped output instead of pretending the stream is contiguous", () => {
    let r = appendTail(emptyRing, tail(lines(1, 3)), RING);
    r = appendTail(r, tail(lines(10, 12), true), RING); // 4..9 fell off the ring
    expect(texts(r)).toEqual(["l1", "l2", "l3", expect.stringMatching(/6 lines were skipped/), "l10", "l11", "l12"]);
    expect(r.last).toBe(12);
  });

  it("does not add a skip marker for the very first tail (nothing was seen before)", () => {
    const r = appendTail(emptyRing, tail(lines(50, 52), true), RING);
    expect(texts(r)).toEqual(["l50", "l51", "l52"]);
  });

  it("puts earlier lines before the ring, drops any that overlap it, and marks a hole", () => {
    let r = appendTail(emptyRing, tail(lines(100, 102)), RING);
    r = prependEarlier(r, lines(10, 14).concat(lines(100, 101)), RING + EARLIER_PAGE);
    expect(texts(r)).toEqual(["l10", "l11", "l12", "l13", "l14", expect.stringMatching(/85 lines were skipped/), "l100", "l101", "l102"]);
  });

  it("earlier lines that are contiguous with the ring get no marker", () => {
    let r = appendTail(emptyRing, tail(lines(5, 6)), RING);
    r = prependEarlier(r, lines(1, 4), RING + EARLIER_PAGE);
    expect(texts(r)).toEqual(["l1", "l2", "l3", "l4", "l5", "l6"]);
  });

  it("keeps the cap with earlier lines loaded: the oldest rows go first", () => {
    let r = appendTail(emptyRing, tail(lines(5001, 5000 + RING)), RING);
    r = prependEarlier(r, lines(1, EARLIER_PAGE), RING + EARLIER_PAGE);
    expect(r.rows.length).toBeLessThanOrEqual(RING + EARLIER_PAGE + 1); // +1: the "skipped" marker row is not a line
    r = appendTail(r, tail(lines(5001 + RING, 5000 + RING + 600)), RING + EARLIER_PAGE);
    expect(r.rows.length).toBeLessThanOrEqual(RING + EARLIER_PAGE);
    expect(r.rows.at(-1)?.text).toBe(`l${5000 + RING + 600}`);
  });

  it("slides past the cap: the newest rows are dropped, the oldest kept, and live output waits", () => {
    let r = appendTail(emptyRing, tail(lines(100001, 100003)), RING);
    let from = 100001;
    for (let k = 0; k < 25; k++) { r = prependEarlier(r, lines(from - EARLIER_PAGE, from - 1), MAX_ROWS); from -= EARLIER_PAGE; }
    expect(r.rows.length).toBe(MAX_ROWS);
    expect(r.rows[0].seq).toBe(from);
    expect(r.detached).toBe(true);
    expect(r.last).toBe(r.rows.at(-1)?.seq);
    expect(appendTail(r, tail(lines(100004, 100005)), MAX_ROWS)).toBe(r);
  });
});
