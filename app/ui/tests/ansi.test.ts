import { describe, expect, it } from "vitest";
import { MAX_LINE, MAX_SEQ, PLAIN, sanitizeLine, sanitizeLines } from "../components/ui/desktop/ansi";

const E = "\u001b";
const text = (raw: string) => sanitizeLine(raw).segments.map((s) => s.text).join("");

describe("ANSI sanitizer (NFR-SEC-3)", () => {
  it("keeps plain text and the 16 SGR colors, bold, dim, italic and underline", () => {
    const r = sanitizeLine(`${E}[1;31merror${E}[0m ok ${E}[94mblue${E}[4munder${E}[24m`);
    expect(r.segments.map((s) => [s.text, s.style.fg, s.style.bold, s.style.underline])).toEqual([
      ["error", 1, true, false],
      [" ok ", null, false, false],
      ["blue", 12, false, false],
      ["under", 12, false, true],
    ]);
  });

  it("drops OSC 8 hyperlinks but keeps the visible label as plain text", () => {
    expect(text(`${E}]8;;https://evil.example/x${E}\\click me${E}]8;;${E}\\`)).toBe("click me");
    expect(text(`${E}]8;;https://evil.example/x\u0007click me${E}]8;;\u0007`)).toBe("click me");
  });

  it("drops OSC 52 clipboard writes, titles and other OSC, BEL and ST terminated", () => {
    expect(text(`a${E}]52;c;ZWNobyBwd25lZA==\u0007b`)).toBe("ab");
    expect(text(`a${E}]52;c;ZWNobyBwd25lZA==${E}\\b`)).toBe("ab");
    expect(text(`a${E}]0;pwned title\u0007b`)).toBe("ab");
  });

  it("drops DCS, APC, PM and SOS strings", () => {
    for (const intro of ["P", "_", "^", "X"]) expect(text(`a${E}${intro}payload${E}\\b`)).toBe("ab");
  });

  it("drops an unterminated control string instead of printing it", () => {
    expect(text(`before${E}]52;c;AAAA`)).toBe("before");
    // No terminator in the line: the rest of the line is the payload and nothing of it is drawn.
    expect(text(`before${E}]52;c;${"A".repeat(MAX_SEQ * 2)}tail`)).toBe("before");
  });

  it("drops an over-limit sequence through its real terminator, drawing none of the payload", () => {
    const payload = "X".repeat(MAX_SEQ + 904);
    expect(text(`${E}]0;${payload}\u0007ok`)).toBe("ok");
    expect(text(`${E}]0;${payload}${E}\\ok`)).toBe("ok");
    expect(text(`a${E}P${payload}${E}\\b`)).toBe("ab");
    // An over-long CSI is dropped to its final byte; an over-long SGR is not applied.
    expect(text(`${E}[${"1;".repeat(MAX_SEQ)}Hok`)).toBe("ok");
    const sgr = sanitizeLine(`${E}[${"0;".repeat(MAX_SEQ)}31mred`);
    expect(sgr.segments.map((x) => [x.text, x.style.fg])).toEqual([["red", null]]);
  });

  it("stays linear on many unterminated or over-long sequences", () => {
    const t0 = Date.now();
    text(`${E}]`.repeat(20000));
    text(`${E}[1;`.repeat(20000));
    expect(Date.now() - t0).toBeLessThan(2000);
  });

  it("drops cursor movement, erase, mode and other CSI sequences", () => {
    for (const seq of ["[2J", "[H", "[10;20H", "[K", "[?1049h", "[?25l", "[5n", "[6n", "[>c", "[1;1R", "[ q", "[?2004h"]) {
      expect(text(`a${E}${seq}b`), seq).toBe("ab");
    }
  });

  it("drops two-character escapes (reset, keypad, charset)", () => {
    for (const seq of ["c", "=", ">", "7", "8", "M", "(0"]) expect(text(`a${E}${seq}b`).startsWith("a"), seq).toBe(true);
    expect(text(`a${E}cb`)).toBe("ab");
  });

  it("drops control characters, C1 introducers, carriage returns and backspaces", () => {
    expect(text("a\u0000b\u0007c\u0008d\u001fe\u007ff\u009bg\u009dh\u0085i")).toBe("abcdefghi");
    expect(text("fake prompt\rreal output")).toBe("fake promptreal output");
    expect(text("a\tb")).toBe("a    b");
  });

  it("shows bidi overrides and zero-width characters as visible escapes", () => {
    expect(text("a‮b")).toBe("a\\u202eb");
    expect(text("a​b﻿c⁦d")).toBe("a\\u200bb\\ufeffc\\u2066d");
  });

  it("escapes more invisible and bidi characters, including those outside the BMP", () => {
    const cp = (n: number) => String.fromCodePoint(n);
    expect(text("x" + cp(0xe0041) + cp(0xe0042) + "y")).toBe("x\\u{e0041}\\u{e0042}y"); // tag characters (hidden ASCII)
    for (const n of [0x061c, 0x00ad, 0x034f, 0x3164, 0xfe0f, 0x2028, 0x2029, 0x180e, 0xe0100]) {
      const out = text("a" + cp(n) + "b");
      expect(out.startsWith("a\\u") && out.endsWith("b"), n.toString(16)).toBe(true);
    }
  });

  it("never splits a surrogate pair at the line cap", () => {
    const r = sanitizeLine("a".repeat(MAX_LINE - 1) + String.fromCodePoint(0x1f600) + "tail");
    const out = r.segments.map((s) => s.text).join("");
    expect(r.truncated).toBe(true);
    expect([...out].length).toBe(MAX_LINE);
    expect(/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(out)).toBe(false);
  });

  it("does not draw 256-color, true-color or background codes, and does not let them eat later codes", () => {
    const r = sanitizeLine(`${E}[38;5;196mA${E}[38;2;1;2;3mB${E}[48;5;21mC${E}[1;38;5;9;31mD`);
    expect(r.segments.map((s) => [s.text, s.style.fg, s.style.bold])).toEqual([["A", null, false], ["B", null, false], ["C", null, false], ["D", 1, true]]);
  });

  it("carries SGR state across lines", () => {
    const l = sanitizeLines([`${E}[32mgreen`, "still green", `${E}[0mplain`]);
    expect(l.map((x) => x.segments[0].style.fg)).toEqual([2, 2, null]);
  });

  it("cuts an oversized line and says so", () => {
    const r = sanitizeLine("x".repeat(MAX_LINE + 500));
    expect(r.truncated).toBe(true);
    expect(r.segments.map((s) => s.text).join("").length).toBe(MAX_LINE);
  });

  it("never throws, always terminates, and never emits a control character (fuzz)", () => {
    const alphabet = [E, "[", "]", "P", "\\", "\u0007", "\u009b", "0", "1", ";", "m", "H", "?", "8", " ", "a", "\r", "\n", "‮", "52", "c"];
    let seed = 12345;
    const rnd = () => (seed = (seed * 1664525 + 1013904223) % 4294967296) / 4294967296;
    for (let n = 0; n < 3000; n++) {
      let raw = "";
      const len = Math.floor(rnd() * 60);
      for (let i = 0; i < len; i++) raw += alphabet[Math.floor(rnd() * alphabet.length)];
      const out = sanitizeLine(raw, PLAIN).segments.map((s) => s.text).join("");
      expect(/[\u0000-\u0008\u000a-\u001f\u007f-\u009f]/.test(out), JSON.stringify(raw)).toBe(false);
      expect(out.includes(E)).toBe(false);
    }
  });

  it("handles a worst-case input of 1 MB of escape introducers in bounded time", () => {
    const t = Date.now();
    sanitizeLine(`${E}[`.repeat(500000));
    sanitizeLine(`${E}]`.repeat(500000));
    expect(Date.now() - t).toBeLessThan(2000);
  });
});
