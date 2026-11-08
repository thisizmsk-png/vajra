// Typed text and pastes to one terminal, in order, in pieces the core accepts. The core never blocks on a shell that is
// not reading: it answers `input_queue_full`, and this waits a moment and retries before dropping the piece.
import { isIpcError, TERM_WRITE_CHUNK } from "../ipc";

export type InputOpts = {
  send: (data: string) => Promise<unknown>;
  /** A piece was dropped because the shell kept not reading. */
  onDrop: () => void;
  /** The terminal is gone (not found, closed). Stops the queue. */
  onGone: () => void;
  retryMs?: number;
  retries?: number;
  sleep?: (ms: number) => Promise<void>;
};

export function createInput(o: InputOpts) {
  const sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const retryMs = o.retryMs ?? 50;
  const retries = o.retries ?? 40;
  let tail: Promise<void> = Promise.resolve();
  let gone = false;
  const sendOne = async (piece: string) => {
    for (let i = 0; i <= retries && !gone; i++) {
      try {
        await o.send(piece);
        return;
      } catch (e) {
        if (isIpcError(e) && e.error === "input_queue_full") {
          if (i === retries) return o.onDrop();
          await sleep(retryMs);
          continue;
        }
        gone = true;
        return o.onGone();
      }
    }
  };
  return {
    push(data: string) {
      let i = 0;
      while (i < data.length) {
        let end = Math.min(i + TERM_WRITE_CHUNK, data.length);
        // Do not split a surrogate pair across two writes.
        if (end < data.length && /[\uD800-\uDBFF]/.test(data[end - 1])) end--;
        const piece = data.slice(i, end);
        i = end;
        tail = tail.then(() => (gone ? undefined : sendOne(piece)));
      }
    },
    /** Resolves when everything pushed so far has been sent or dropped. */
    flush: () => tail,
  };
}
