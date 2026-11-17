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

/** What a Director-General or Lead did to a worker through the core's tools. */
export type WorkerAction = "message" | "redirect" | "inspect" | "stop";
/** One tool line of a `tool` turn. `worker` is set when the call named a worker. */
export type ToolCall = { line: string; tool: string; worker: string | null; action: WorkerAction; text: string | null };

const WORKER_TOOLS: Record<string, WorkerAction> = { message_worker: "message", redirect_worker: "redirect", inspect_worker: "inspect", stop_worker: "stop" };
/** A JSON string field, read from text that may be cut short (the core keeps the first 200 characters of the input). */
const field = (json: string, name: string): string | null => {
  const m = new RegExp(`"${name}"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)("|…?$)`).exec(json);
  if (!m) return null;
  try { return JSON.parse(`"${m[1].replace(/\\$/, "")}"`) as string; } catch { return m[1]; }
};

/** The lines of a `tool` turn, each read as `<tool> <json input>` (desktop lead stream). The tool may carry its MCP
 *  server prefix (`mcp__vajra-desktop__message_worker`). Plain text, never markup. */
export function toolCalls(text: string): ToolCall[] {
  return text.split("\n").filter((l) => l.trim() !== "").map((line) => {
    const sp = line.indexOf(" ");
    const tool = (sp > 0 ? line.slice(0, sp) : line).split("__").pop() ?? "";
    const action = WORKER_TOOLS[tool];
    const input = sp > 0 ? line.slice(sp + 1) : "";
    const worker = action ? field(input, "worker") : null;
    return { line, tool, worker: worker || null, action: action ?? "inspect", text: action === "message" || action === "redirect" ? field(input, "text") : null };
  });
}
