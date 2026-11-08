// Dev only (loaded by devFixture.ts): a pretend shell behind the term_* commands, so the dock can be seen and clicked
// through with `?fixture#terminal` and no core. It echoes what is typed and answers a few lines.
import { setTransport } from "../ipc";
import type { Transport } from "../ipc";

type Sess = { send: (m: unknown) => void; line: string };

export function terminalFixture(base: Transport): Transport {
  const sessions = new Map<number, Sess>();
  let next = 1;
  const enc = new TextEncoder();
  const out = (s: Sess, text: string) => s.send(enc.encode(text).slice().buffer);
  const prompt = (s: Sess) => out(s, "\u001b[32mfixture\u001b[0m $ ");
  const info = (id: number, cwd: string) => ({ id, cwd, root: cwd, rows: 24, cols: 80, alive: true });
  const run = (id: number, s: Sess, line: string) => {
    if (line === "exit") return s.send({ exit: 0 });
    if (line === "flood") out(s, Array.from({ length: 400 }, (_, i) => `line ${i + 1} of the flood`).join("\r\n") + "\r\n");
    else if (line) out(s, `fixture: no such command: ${line}\r\n`);
    prompt(s);
    void id;
  };
  return {
    ...base,
    async call(cmd, args) {
      if (cmd === "term_list") return { terminals: [] };
      if (cmd === "term_write") {
        const id = Number(args?.id);
        const s = sessions.get(id);
        if (!s) throw { error: "not_found", detail: "no such terminal" };
        for (const ch of String(args?.data)) {
          if (ch === "\r") {
            out(s, "\r\n");
            const line = s.line.trim();
            s.line = "";
            run(id, s, line);
          } else if (ch === "\u007f") {
            if (s.line) { s.line = s.line.slice(0, -1); out(s, "\b \b"); }
          } else if (ch >= " ") {
            s.line += ch;
            out(s, ch);
          }
        }
        return {};
      }
      if (cmd === "term_close") { sessions.delete(Number(args?.id)); return {}; }
      if (cmd === "term_resize" || cmd === "term_ack") return {};
      return base.call(cmd, args);
    },
    async channel(cmd, args, onMessage) {
      if (cmd === "term_open") {
        const cwd = String(args.cwd);
        if (cwd.includes("denied")) throw { error: "cwd_not_allowed", detail: "not a root" };
        const id = next++;
        const s: Sess = { send: onMessage, line: "" };
        sessions.set(id, s);
        setTimeout(() => { out(s, `Fixture shell in ${cwd}. Type flood or exit.\r\n`); prompt(s); }, 0);
        return info(id, cwd);
      }
      throw { error: "not_in_fixture", detail: cmd };
    },
  };
}

export const installTerminalFixture = (base: Transport) => setTransport(terminalFixture(base));
