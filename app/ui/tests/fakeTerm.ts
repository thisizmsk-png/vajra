import type { TermInfo } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";

/** A recording backend for user terminals: `term_open`/`term_attach` stream to a channel the test can push to. */
export function fakeTerm(over: Record<string, unknown | ((a?: Record<string, unknown>) => unknown)> = {}) {
  let n = 0;
  const info = (cwd: string): TermInfo => ({ id: ++n, cwd, root: cwd, rows: 24, cols: 80, alive: true });
  const ipc = fakeIpc({ term_write: {}, term_resize: {}, term_ack: {}, term_close: {}, term_list: { terminals: [] }, ...over });
  const streams: { cmd: string; id: number; send: (m: unknown) => void }[] = [];
  ipc.transport.channel = async (cmd, args, onMessage) => {
    ipc.calls.push({ cmd, args });
    const r = (over[cmd] ?? ((a?: Record<string, unknown>) => (cmd === "term_open" ? info(String(a?.cwd)) : { ...info("/x"), id: a?.id })))
    const v = typeof r === "function" ? (r as (a?: Record<string, unknown>) => unknown)(args) : r;
    if (v instanceof Error) throw v;
    streams.push({ cmd, id: (v as TermInfo).id, send: onMessage });
    return v;
  };
  const named = (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd);
  return { ...ipc, streams, named, bytes: (s: string | number[]) => (typeof s === "string" ? new TextEncoder().encode(s) : Uint8Array.from(s)) };
}

/** A stand-in for the xterm.js Terminal: no layout or canvas in jsdom, so tests drive it directly. */
export class FakeXterm {
  static all: FakeXterm[] = [];
  rows = 24;
  cols = 80;
  options: Record<string, unknown> = {};
  textarea = document.createElement("textarea");
  written: Uint8Array[] = [];
  buffer = { active: { type: "normal" as "normal" | "alternate" } };
  focused = 0;
  disposed = false;
  selection = "";
  pasted: string[] = [];
  private data: Array<(d: string) => void> = [];
  private resizeCb: Array<(s: { rows: number; cols: number }) => void> = [];
  keyHandler: ((e: KeyboardEvent) => boolean) | null = null;
  constructor(opts: Record<string, unknown>) {
    this.options = opts;
    FakeXterm.all.push(this);
  }
  loadAddon() {}
  open(el: HTMLElement) {
    el.appendChild(this.textarea);
  }
  write(d: Uint8Array, cb?: () => void) {
    this.written.push(d);
    cb?.();
  }
  onData(f: (d: string) => void) {
    this.data.push(f);
  }
  onResize(f: (s: { rows: number; cols: number }) => void) {
    this.resizeCb.push(f);
  }
  attachCustomKeyEventHandler(f: (e: KeyboardEvent) => boolean) {
    this.keyHandler = f;
  }
  focus() {
    this.focused++;
  }
  getSelection() {
    return this.selection;
  }
  paste(t: string) {
    this.pasted.push(t);
  }
  dispose() {
    this.disposed = true;
  }
  type(d: string) {
    this.data.forEach((f) => f(d));
  }
  resize(rows: number, cols: number) {
    this.rows = rows;
    this.cols = cols;
    this.resizeCb.forEach((f) => f({ rows, cols }));
  }
  text() {
    return this.written.map((b) => new TextDecoder().decode(b)).join("");
  }
}
