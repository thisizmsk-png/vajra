// The Director-General's stream as turns. The core writes one bus line per text line, `<tag>\t<text>`, with the tags
// `you` (your question), `lead` (its answer), `tool` (a tool it used) and `error` (desktop/lead.rs). Consecutive lines
// with one tag are one turn. A line with no known tag is its answer (older streams). Text is plain: never parsed as markup.
export type Role = "you" | "dg" | "tool" | "error";
export type Turn = { role: Role; text: string };

const TAGS: Record<string, Role> = { you: "you", lead: "dg", tool: "tool", error: "error" };

export function turns(lines: string[]): Turn[] {
  const out: Turn[] = [];
  for (const line of lines) {
    const tab = line.indexOf("\t");
    const tag = tab > 0 ? TAGS[line.slice(0, tab)] : undefined;
    const role: Role = tag ?? "dg";
    const text = tag ? line.slice(tab + 1) : line;
    const last = out[out.length - 1];
    if (last && last.role === role && role !== "you") last.text += `\n${text}`;
    else out.push({ role, text });
  }
  return out;
}
