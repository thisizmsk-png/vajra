/**
 * A character that could make a command show differently from what runs. The core refuses these before it ever asks
 * (the approval module's `hides_text`); this is the dialog's own second check, so a list that somehow carries one
 * can't be approved here:
 * - any control character but tab (Cc), the line and paragraph separators (Zl, Zp), every Default_Ignorable_Code_Point;
 * - every format character (Cf), private use (Co) and unassigned code point or noncharacter (Cn);
 * - the combining marks that enclose (Me) or overlay (canonical combining class 1) the character before them.
 * Every code point here is written as an escape (braced form), so the source itself has no invisible character.
 */
const HIDES = /[\p{Cc}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}\p{Cf}\p{Co}\p{Cn}\p{Me}\u{0334}-\u{0338}\u{1CD4}\u{1CE2}-\u{1CE8}\u{20D2}\u{20D3}\u{20D8}-\u{20DA}\u{20E5}\u{20E6}\u{20EA}\u{20EB}\u{10A39}\u{16AF0}-\u{16AF4}\u{1BC9E}\u{1D167}-\u{1D169}]/u;
export const hidesText = (s: string) => HIDES.test(s.replace(/\t/g, ""));

/** Splits `s` into plain-ASCII runs and single non-ASCII characters (bhishma L3): a lookalike letter is then visible. */
export function asciiRuns(s: string): Array<{ text: string; ascii: boolean }> {
  const out: Array<{ text: string; ascii: boolean }> = [];
  for (const ch of s) {
    const ascii = ch.codePointAt(0)! < 0x80;
    const last = out[out.length - 1];
    if (ascii && last?.ascii) last.text += ch;
    else out.push({ text: ch, ascii });
  }
  return out;
}

/** `U+0430` style name of a single character. */
export const codePoint = (ch: string) => `U+${ch.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}`;
