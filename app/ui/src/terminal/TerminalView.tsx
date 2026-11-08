// SPIKE: one xterm.js view bound to one core PTY. Loaded lazily (see TerminalDock) so the xterm code and its memory are
// only paid for once a terminal is opened.
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { api } from "../ipc";

/** Acknowledge parsed output to the core in batches, so a flood is paced by what xterm has actually rendered. */
const ACK_EVERY = 64 * 1024;

export default function TerminalView({ cwd, active, onExit }: { cwd: string; active: boolean; onExit: (code: number | null) => void }) {
  const host = useRef<HTMLDivElement>(null);
  const fit = useRef<FitAddon | null>(null);
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const term = new Terminal({ scrollback: 5000, cursorBlink: true, fontFamily: "ui-monospace, Menlo, monospace", fontSize: 12, allowProposedApi: false });
    const fitter = new FitAddon();
    fit.current = fitter;
    term.loadAddon(fitter);
    term.open(el);
    fitter.fit();
    let id: number | null = null;
    let dead = false;
    let unacked = 0;
    const ack = (n: number) => {
      unacked += n;
      if (id !== null && unacked >= ACK_EVERY) {
        void api.termAck(id, unacked);
        unacked = 0;
      }
    };
    void api
      .termOpen(cwd, term.rows, term.cols, (b) => term.write(b, () => ack(b.length)), (code) => { term.write(`\r\n[process exited${code === null ? "" : ` with code ${code}`}]\r\n`); onExit(code); })
      .then((i) => {
        if (dead) { void api.termClose(i); return; }
        id = i;
        term.onData((d) => void api.termWrite(i, d)); // typed text and pastes go to the PTY as they are, never through a shell command line
        term.onResize(({ rows, cols }) => void api.termResize(i, rows, cols));
      });
    const ro = new ResizeObserver(() => fitter.fit());
    ro.observe(el);
    return () => {
      dead = true;
      ro.disconnect();
      if (id !== null) void api.termClose(id); // closing the tab ends the shell and its session
      term.dispose();
    };
  }, [cwd]);
  useEffect(() => { if (active) fit.current?.fit(); }, [active]);
  return <div ref={host} className="v-term__host" style={{ display: active ? "block" : "none" }} />;
}
