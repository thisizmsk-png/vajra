/**
 * A character that could make a command show differently from what runs: any control character but tab, the line and
 * paragraph separators (Zl and Zp, whose only members are U+2028 and U+2029), and every Default_Ignorable_Code_Point.
 * The core refuses these before it ever asks; this is the dialog's own second check, so a list that somehow carries
 * one can't be approved here.
 */
const HIDES = /[\p{Cc}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}]/u;
export const hidesText = (s: string) => HIDES.test(s.replace(/\t/g, ""));
