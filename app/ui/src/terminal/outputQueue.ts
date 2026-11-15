// The output budget of one terminal view (FG-12 S12.5). Bytes from the core are not handed to xterm as they arrive:
//   * they are coalesced, at most one xterm write per animation frame and none while the previous one is still being
//     parsed, so a flood costs a bounded amount of parsing and painting per frame, not per message;
//   * a frame carries at most `frameBytes`;
//   * if the view falls far behind (`maxPending`), the oldest bytes are dropped and a "skipped N KB" line marks the gap
//     (the scrollback keeps only the last lines anyway, so what is dropped would have scrolled away unread);
//   * a hidden pane renders nothing: it holds at most `hiddenCap` bytes of the newest output and writes them when it is
//     shown. Dropped bytes are acknowledged at once, so the core never stalls on output nobody will see.
// Bytes are acknowledged to the core only when xterm has parsed them (or when they are dropped), so the core's credit
// window keeps pacing the shell.

export type QueueOptions = {
  /** Hand bytes to xterm; call `done` once it has parsed them. */
  write: (data: Uint8Array, done: () => void) => void;
  /** Bytes the core may send more of (parsed or dropped). */
  ack: (bytes: number) => void;
  /** Run `fn` at the next frame; returns a cancel. */
  schedule: (fn: () => void) => () => void;
  /** What to write where `bytes` were dropped. */
  marker: (bytes: number) => Uint8Array;
  frameBytes?: number;
  maxPending?: number;
  hiddenCap?: number;
};

export const FRAME_BYTES = 64 * 1024;
export const MAX_PENDING = 512 * 1024;
export const HIDDEN_CAP = 256 * 1024;
/** After a drop, start the kept output at a line boundary if one is this close to the cut. */
const ALIGN = 4096;

export type OutputQueue = {
  push: (data: Uint8Array) => void;
  setVisible: (visible: boolean) => void;
  dispose: () => void;
  /** For tests and diagnostics. */
  stats: () => { pending: number; dropped: number };
};

export function createOutputQueue(o: QueueOptions): OutputQueue {
  const frameBytes = o.frameBytes ?? FRAME_BYTES;
  const maxPending = o.maxPending ?? MAX_PENDING;
  const hiddenCap = o.hiddenCap ?? HIDDEN_CAP;
  let chunks: Uint8Array[] = [];
  let pending = 0;
  let visible = true;
  let writing = false;
  let dead = false;
  let cancel: (() => void) | null = null;
  let skipped = 0; // dropped since the last marker was written
  let droppedTotal = 0;

  const dropToTail = (cap: number) => {
    if (pending <= cap) return;
    let drop = pending - cap;
    let n = 0;
    while (drop > 0 && chunks.length) {
      const c = chunks[0];
      if (c.length <= drop) {
        chunks.shift();
        drop -= c.length;
        n += c.length;
      } else {
        chunks[0] = c.subarray(drop);
        n += drop;
        drop = 0;
      }
    }
    // Begin the kept output at a line start when one is close, so the cut does not land mid-line.
    const head = chunks[0];
    if (head) {
      const nl = head.subarray(0, Math.min(ALIGN, head.length)).indexOf(10);
      if (nl >= 0) {
        if (nl + 1 === head.length) chunks.shift();
        else chunks[0] = head.subarray(nl + 1);
        n += nl + 1;
      }
    }
    pending -= n;
    skipped += n;
    droppedTotal += n;
    o.ack(n);
  };

  const take = (max: number): Uint8Array => {
    const out = new Uint8Array(Math.min(max, pending));
    let at = 0;
    while (at < out.length && chunks.length) {
      const c = chunks[0];
      const k = Math.min(c.length, out.length - at);
      out.set(c.subarray(0, k), at);
      at += k;
      if (k === c.length) chunks.shift();
      else chunks[0] = c.subarray(k);
    }
    pending -= out.length;
    return out;
  };

  const frame = () => {
    cancel = null;
    if (dead || !visible || writing || pending === 0) return;
    if (skipped > 0) {
      o.write(o.marker(skipped), () => undefined);
      skipped = 0;
    }
    const data = take(frameBytes);
    writing = true;
    o.write(data, () => {
      writing = false;
      if (dead) return;
      o.ack(data.length);
      arm();
    });
  };

  const arm = () => {
    if (dead || !visible || writing || pending === 0 || cancel) return;
    cancel = o.schedule(frame);
  };

  return {
    push(data) {
      if (dead || data.length === 0) return;
      chunks.push(data);
      pending += data.length;
      dropToTail(visible ? maxPending : hiddenCap);
      arm();
    },
    setVisible(v) {
      if (visible === v) return;
      visible = v;
      if (v) arm();
      else {
        cancel?.();
        cancel = null;
        dropToTail(hiddenCap);
      }
    },
    dispose() {
      dead = true;
      cancel?.();
      cancel = null;
      chunks = [];
      pending = 0;
    },
    stats: () => ({ pending, dropped: droppedTotal }),
  };
}

/** One call per animation frame; in a window with no animation frames (a background webview, jsdom) a 16 ms timer. */
export const frameScheduler = (fn: () => void): (() => void) => {
  if (typeof requestAnimationFrame === "function") {
    const id = requestAnimationFrame(() => fn());
    return () => cancelAnimationFrame(id);
  }
  const id = setTimeout(fn, 16);
  return () => clearTimeout(id);
};
