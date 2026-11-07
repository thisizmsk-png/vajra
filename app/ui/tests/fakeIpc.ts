import type { Transport } from "../src/ipc";

/** A recording transport: `replies[cmd]` is a value or a function of the args; a missing command rejects. */
export function fakeIpc(replies: Record<string, unknown | ((args?: Record<string, unknown>) => unknown)> = {}) {
  const calls: { cmd: string; args?: Record<string, unknown> }[] = [];
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const transport: Transport = {
    async call(cmd, args) {
      calls.push({ cmd, args });
      if (!(cmd in replies)) throw { error: "unknown_command", detail: cmd };
      const r = replies[cmd];
      const v = typeof r === "function" ? (r as (a?: Record<string, unknown>) => unknown)(args) : r;
      if (v instanceof Error) throw v;
      return v;
    },
    on(event, cb) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event)!.add(cb);
      return () => listeners.get(event)!.delete(cb);
    },
  };
  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((cb) => cb(payload));
  return { transport, calls, emit, listenerCount: (e: string) => listeners.get(e)?.size ?? 0 };
}
