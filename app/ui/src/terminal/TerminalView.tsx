// One xterm.js view bound to one core PTY. Loaded lazily by the dock, so the xterm code and its memory are paid for
// only once a terminal is opened. DOM renderer only: the WebGL addon would start a GPU helper process (about 32 MiB).
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { S } from "../../strings";
import { api, errorText, isIpcError } from "../ipc";
import type { TermMsg } from "../ipc";
import { createInput } from "./input";
import { createOutputQueue, frameScheduler } from "./outputQueue";

/** Output is acknowledged to the core in steps like this, once xterm has parsed it: a flood is paced by the view. */
export const ACK_EVERY = 64 * 1024;
const SCROLLBACK = 2000;

export type ViewState =
  | { status: "running"; id: number }
  | { status: "ended"; code: number | null }
  | { status: "error"; code: string; detail: string }
  | { status: "busy" };

export type ViewProps = {
  cwd: string;
  /** Run this catalog harness's own sign-in instead of a shell (the core builds the command from the id). */
  signIn?: string;
  /** Re-bind a terminal that outlived the webview instead of opening a new one. */
  attachId?: number;
  active: boolean;
  /** Bumped by the dock to focus the terminal (dock shown, tab selected). */
  focusToken: number;
  onState: (s: ViewState) => void;
  /** New output arrived while this pane was hidden (the core reports it at most twice a second). */
  onActivity?: () => void;
};

const token = (name: string, fallback: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
const theme = () => ({
  background: token("--c-background", "Canvas"),
  foreground: token("--c-text", "CanvasText"),
  cursor: token("--c-primary", "CanvasText"),
  selectionBackground: token("--c-selection", "Highlight"),
});

export default function TerminalView({ cwd, signIn, attachId, active, focusToken, onState, onActivity }: ViewProps) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const queueRef = useRef<ReturnType<typeof createOutputQueue> | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const onStateRef = useRef(onState);
  onStateRef.current = onState;
  const onActivityRef = useRef(onActivity);
  onActivityRef.current = onActivity;
  const idRef = useRef<number | null>(null);
  const sentVisible = useRef<boolean | null>(null);
  const tellRef = useRef<((id: number, visible: boolean) => void) | null>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const t = new Terminal({
      scrollback: SCROLLBACK,
      cursorBlink: true,
      fontFamily: token("--font-mono", "ui-monospace, Menlo, monospace"),
      fontSize: 12,
      theme: theme(),
      allowProposedApi: false,
    });
    const fitter = new FitAddon();
    t.loadAddon(fitter);
    t.open(el);
    t.textarea?.setAttribute("aria-label", S.terminal.inputLabel);
    term.current = t;
    fit.current = fitter;
    const refit = () => {
      // A hidden dock measures as zero: keep the last size instead of fitting to nothing.
      if (el.offsetWidth > 0 && el.offsetHeight > 0) fitter.fit();
    };
    refit();

    let id: number | null = null;
    let dead = false;
    const tellVisible = (to: number, visible: boolean) => {
      sentVisible.current = visible;
      void api.termVisible(to, visible, t.buffer.active.type === "alternate").catch(() => undefined);
    };
    tellRef.current = tellVisible;
    let unacked = 0;
    // Bytes this view puts through the queue itself (the "skipped" line, a screen switch) were never sent by the core, so
    // they are not acknowledged to it: the total acknowledged stays equal to the total sent.
    let own = 0;
    const input = createInput({
      send: (d) => api.termWrite(id as number, d),
      onDrop: () => onStateRef.current({ status: "busy" }),
      onGone: () => undefined,
    });
    const ack = (n: number) => {
      const mine = Math.min(own, n);
      own -= mine;
      n -= mine;
      unacked += n;
      if (id !== null && unacked >= ACK_EVERY) {
        const k = unacked;
        unacked = 0;
        void api.termAck(id, k).catch(() => undefined);
      }
    };
    // Output goes through the frame budget: one write per frame, drop-to-tail when far behind, nothing rendered while the
    // pane is hidden (see outputQueue.ts). `ack` still waits for xterm to parse, so the core's credit window paces the shell.
    const queue = createOutputQueue({
      write: (d, done) => t.write(d, done),
      ack,
      schedule: frameScheduler,
      marker: (n) => new TextEncoder().encode(`\x1b[0m\r\n${S.terminal.skipped(Math.max(1, Math.round(n / 1024)))}\r\n`),
    });
    queue.setVisible(activeRef.current);
    queueRef.current = queue;
    const onMsg = (m: TermMsg) => {
      // A disposed view (StrictMode's first setup, a closed tab) may still get messages on its old channel, such as the
      // Exit the core sends when it closes that shell. They belong to a terminal this view no longer represents.
      if (dead) return;
      if ("data" in m) queue.push(m.data);
      else if ("exit" in m) onStateRef.current({ status: "ended", code: m.exit });
      else if ("activity" in m) onActivityRef.current?.();
      else {
        // The pane was hidden and is shown: what the core left out, and a screen switch if the program made one meanwhile.
        const enc = new TextEncoder();
        const put = (text: string) => {
          const b = enc.encode(text);
          own += b.length;
          queue.push(b);
        };
        if (m.alt === false) put("\x1b[?1049l");
        if (m.alt === true) put("\x1b[?1049h");
        if (m.skipped > 0) put(`\x1b[0m\r\n${S.terminal.skipped(Math.max(1, Math.round(m.skipped / 1024)))}\r\n`);
      }
    };
    const started = attachId !== undefined ? api.termAttach(attachId, onMsg) : signIn !== undefined ? api.setupSignIn(signIn, t.rows || 24, t.cols || 80, onMsg) : api.termOpen(cwd, t.rows || 24, t.cols || 80, onMsg);
    started.then(
      (info) => {
        if (dead) {
          // This view was disposed before the answer came back. A terminal it opened itself is its own to end; one it
          // only attached to belongs to whoever attached last (under StrictMode the next setup has already re-attached
          // it), so closing it here would kill a live shell.
          if (attachId === undefined) void api.termClose(info.id).catch(() => undefined);
          return;
        }
        id = info.id;
        idRef.current = info.id;
        // The core starts a pane as visible; tell it if this one is not (a tab opened in the background).
        if (!activeRef.current) tellVisible(info.id, false);
        // Typed text and pastes go to the PTY as they are, never through a command line.
        t.onData((d) => input.push(d));
        t.onResize(({ rows, cols }) => void api.termResize(info.id, rows, cols).catch(() => undefined));
        if (attachId !== undefined && (t.rows !== info.rows || t.cols !== info.cols)) void api.termResize(info.id, t.rows, t.cols).catch(() => undefined);
        onStateRef.current(info.alive ? { status: "running", id: info.id } : { status: "ended", code: null });
      },
      (e) => {
        if (!dead) onStateRef.current({ status: "error", code: isIpcError(e) ? e.error : "unknown", detail: errorText(e) });
      },
    );

    // Copy and paste. A plain Ctrl+C always reaches the shell as an interrupt; the copy chord adds Shift. Command+C and
    // Command+V on macOS are handled by xterm and the browser themselves.
    t.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      if (e.ctrlKey && e.key === "`") return false; // the dock's toggle; it listens on the window
      if (e.ctrlKey && e.shiftKey && e.code === "KeyC") {
        const sel = t.getSelection();
        if (sel) void navigator.clipboard?.writeText(sel).catch(() => undefined);
        return false;
      }
      if (e.ctrlKey && e.shiftKey && e.code === "KeyV") {
        void navigator.clipboard?.readText().then((text) => text && t.paste(text)).catch(() => undefined);
        return false;
      }
      return true;
    });

    const ro = new ResizeObserver(refit);
    ro.observe(el);
    const themeWatch = new MutationObserver(() => (t.options.theme = theme()));
    themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      dead = true;
      queue.dispose();
      queueRef.current = null;
      idRef.current = null;
      sentVisible.current = null;
      tellRef.current = null;
      ro.disconnect();
      themeWatch.disconnect();
      if (id !== null) void api.termClose(id).catch(() => undefined); // closing a tab ends the shell and its session
      t.dispose();
      term.current = null;
      fit.current = null;
    };
  }, [cwd, signIn, attachId]);

  useEffect(() => {
    queueRef.current?.setVisible(active);
    // The core keeps a hidden pane's output (newest 256 KiB) instead of sending it. Before the shell is known the start-up
    // path tells it; after that every change is sent.
    if (idRef.current !== null && sentVisible.current !== active) tellRef.current?.(idRef.current, active);
    if (!active) return;
    const el = host.current;
    if (el && el.offsetWidth > 0) fit.current?.fit();
  }, [active]);
  useEffect(() => {
    if (active && focusToken > 0) term.current?.focus();
  }, [active, focusToken]);

  return <div ref={host} className="v-term__host" hidden={!active} />;
}
