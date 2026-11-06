// NFR-SEC-3: agent output is untrusted text. This turns one raw terminal line into styled text segments and nothing
// else: no HTML, no links, no clipboard, no cursor movement. Only SGR (colors 0-15, bold, dim, italic, underline)
// survives; every other escape sequence and control character is dropped, and bidi or zero-width characters are
// shown as visible escapes.

export type Style = { fg: number | null; bold: boolean; dim: boolean; italic: boolean; underline: boolean };
export type Segment = { text: string; style: Style };

export const PLAIN: Style = { fg: null, bold: false, dim: false, italic: false, underline: false };
export const MAX_LINE = 8192; // code points kept per line; the rest is cut and marked
export const MAX_SEQ = 4096; // an escape sequence longer than this is dropped whole, through its real terminator, and never applied

const ESC = 0x1b;
const BEL = 0x07;
// Characters that change how text reads, or hide data, without being drawn: shown as visible escapes. A table of code point
// ranges (not a regex over source characters), matched per code point, so characters outside the BMP are covered too.
const INVISIBLE: ReadonlyArray<readonly [number, number]> = [
  [0x00ad, 0x00ad], [0x034f, 0x034f], [0x061c, 0x061c], [0x115f, 0x1160], [0x17b4, 0x17b5], [0x180b, 0x180e],
  [0x200b, 0x200f], [0x2028, 0x202e], [0x2060, 0x206f], [0x3164, 0x3164], [0xfe00, 0xfe0f], [0xfeff, 0xfeff],
  [0xffa0, 0xffa0], [0xe0000, 0xe007f], [0xe0100, 0xe01ef],
];
const isInvisible = (cp: number) => INVISIBLE.some(([a, b]) => cp >= a && cp <= b);

const visible = (cp: number) => (cp > 0xffff ? "\\u{" + cp.toString(16) + "}" : "\\u" + cp.toString(16).padStart(4, "0"));

function applySgr(style: Style, params: string): Style {
  let s = { ...style };
  const codes = params === "" ? [0] : params.split(/[;:]/).map((p) => (p === "" ? 0 : Number(p)));
  for (let i = 0; i < codes.length; i++) {
    const c = codes[i];
    if (!Number.isFinite(c)) continue;
    if (c === 0) s = { ...PLAIN };
    else if (c === 1) s.bold = true;
    else if (c === 2) s.dim = true;
    else if (c === 3) s.italic = true;
    else if (c === 4) s.underline = true;
    else if (c === 22) { s.bold = false; s.dim = false; }
    else if (c === 23) s.italic = false;
    else if (c === 24) s.underline = false;
    else if (c >= 30 && c <= 37) s.fg = c - 30;
    else if (c >= 90 && c <= 97) s.fg = c - 90 + 8;
    else if (c === 39) s.fg = null;
    else if (c === 38 || c === 48 || c === 58) i += codes[i + 1] === 5 ? 2 : codes[i + 1] === 2 ? 4 : 0; // skip 256/true-color
    // backgrounds and everything else are ignored: only the 16 contrast-checked foregrounds are drawn
  }
  return s;
}

/**
 * Skip an escape sequence that starts at `i` (a control string ends at BEL or ST). Returns [next index, sgr params|null].
 * The scan is never cut at MAX_SEQ: a sequence that long is dropped through its real terminator (so no payload is drawn
 * as text), just not applied. With no terminator in the line, everything to the end of the line is dropped.
 */
function skipEscape(raw: string, i: number): [number, string | null] {
  const next = raw[i + 1];
  if (next === "[") {
    let j = i + 2;
    while (j < raw.length && raw.charCodeAt(j) >= 0x30 && raw.charCodeAt(j) <= 0x3f) j++; // parameter bytes
    while (j < raw.length && raw.charCodeAt(j) >= 0x20 && raw.charCodeAt(j) <= 0x2f) j++; // intermediate bytes
    if (j < raw.length && raw.charCodeAt(j) >= 0x40 && raw.charCodeAt(j) <= 0x7e) {
      const sgr = raw[j] === "m" && j + 1 - i <= MAX_SEQ ? raw.slice(i + 2, j) : null;
      return [j + 1, sgr];
    }
    return [Math.max(j, i + 2), null]; // malformed (or cut off by the end of the line): drop what was scanned
  }
  if (next === "]" || next === "P" || next === "X" || next === "^" || next === "_") {
    // OSC (incl. OSC 8 hyperlinks and OSC 52 clipboard), DCS, SOS, PM, APC: dropped through BEL or ST.
    for (let j = i + 2; j < raw.length; j++) {
      const c = raw.charCodeAt(j);
      if (c === BEL) return [j + 1, null];
      if (c === ESC && raw[j + 1] === "\\") return [j + 2, null];
    }
    return [raw.length, null]; // unterminated: the rest of the line is the payload
  }
  return [Math.min(raw.length, i + 2), null]; // a two-character escape (ESC c, ESC =, ...)
}

/** One raw line (no newline) to styled text segments. Never throws; always terminates. */
export function sanitizeLine(raw: string, start: Style = PLAIN): { segments: Segment[]; end: Style; truncated: boolean } {
  const segments: Segment[] = [];
  let style = start;
  let buf = "";
  let kept = 0;
  let truncated = false;
  const flush = () => {
    if (buf) segments.push({ text: buf, style });
    buf = "";
  };
  for (let i = 0; i < raw.length; ) {
    const c = raw.charCodeAt(i);
    if (c === ESC) {
      const [next, sgr] = skipEscape(raw, i);
      if (sgr !== null) {
        flush();
        style = applySgr(style, sgr);
      }
      i = next;
      continue;
    }
    // C1 controls (8-bit CSI/OSC/DCS introducers included) and C0 controls except tab are dropped.
    if ((c < 0x20 && c !== 0x09) || (c >= 0x7f && c <= 0x9f)) {
      i++;
      continue;
    }
    const cp = raw.codePointAt(i)!; // a whole code point: a surrogate pair is never split
    const ch = String.fromCodePoint(cp);
    i += ch.length;
    if (kept >= MAX_LINE) {
      truncated = true;
      break;
    }
    buf += isInvisible(cp) ? visible(cp) : c === 0x09 ? "    " : ch;
    kept++;
  }
  flush();
  return { segments, end: style, truncated };
}

/** Lines of a scrollback buffer; SGR state carries from one line to the next, as in a terminal. */
export function sanitizeLines(lines: readonly string[]): Array<{ segments: Segment[]; truncated: boolean }> {
  let style = PLAIN;
  return lines.map((l) => {
    const r = sanitizeLine(l, style);
    style = r.end;
    return { segments: r.segments, truncated: r.truncated };
  });
}
