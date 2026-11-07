// Perf-CI fixture. Compiled in ONLY when the build sets VITE_PERF_FIXTURE (the CI post-load measurement build); the
// normal bundle contains none of this (CI greps the shipped bundle for it). It replaces the core with a scripted one:
// a Crew view with one worker whose stream starts LOAD_START_S after launch and delivers TOTAL lines at RATE per second,
// the way a long run would, so the app's post-load idle can be measured. Mirrors LOAD_START_S/LOAD_END_S in
// app/bench/ci/measure.py.
import { setTransport } from "../ipc";
import type { Transport } from "../ipc";

export const LOAD_START_S = 20;
export const RATE = 170;
export const TOTAL = 10_000;
export const LOAD_END_S = LOAD_START_S + Math.ceil(TOTAL / RATE);
/** What the real core keeps in memory for a stream; older lines are in its spill file. */
const CORE_RING = 2900;
const PAGE = 700;

const pad = "the quick brown fox jumps over the lazy dog while the crew builds the thing ";
const line = (n: number) => `\x1b[32m${String(n).padStart(6, "0")}\x1b[0m Edit src/api/reports.py +${n % 40} -${n % 7} ${pad.slice(0, 20 + (n % 40))}`;
const lines = (a: number, b: number) => Array.from({ length: Math.max(0, b - a + 1) }, (_, i) => ({ seq: a + i, text: line(a + i) }));
const node = (id: string, kind: string, title: string, children: unknown[] = []) =>
  ({ id, kind, title, engine_id: null, status: "in_progress", overlays: [], children });

/** Lines delivered so far, `elapsedS` seconds after launch. */
export const delivered = (elapsedS: number) => Math.max(0, Math.min(TOTAL, Math.floor((elapsedS - LOAD_START_S) * RATE)));

export function fixtureTransport(elapsedS: () => number): { transport: Transport; tick: () => void } {
  const listeners = new Set<(p: unknown) => void>();
  const transport: Transport = {
    async call(cmd, a) {
      const args = (a ?? {}) as Record<string, number>;
      const c = delivered(elapsedS());
      if (cmd === "crew") return node("lead", "lead", "Lead", [node("run", "run", "Run r1", [node("agent:a1", "worker", "Backend")])]);
      if (cmd === "active_runs") return { count: 0, runs: [] };
      if (cmd === "worktree_info") return { worker: "a1", worktree: "/fixture/.claude/worktrees/w1", branch: "worker-1", base: null };
      if (cmd === "tail") {
        const after = args.afterSeq ?? 0;
        const spilled = Math.max(0, c - CORE_RING);
        const gap = c - after > CORE_RING;
        const from = gap ? c - PAGE + 1 : after + 1;
        return {
          lines: lines(from, Math.min(c, from + PAGE - 1)), gap, head_seq: c,
          ...(spilled > 0 ? { spill_start: 0, spill_end: spilled * 100 } : { spill_start: null, spill_end: null }),
        };
      }
      if (cmd === "read_spill") {
        const spilled = Math.max(0, c - CORE_RING);
        const from = Math.floor((args.offset ?? 0) / 100) + 1;
        const to = Math.min(spilled, from + PAGE - 1);
        return { lines: lines(from, to), next_offset: to * 100, gap: false, end: to >= spilled };
      }
      return {};
    },
    on(_e, cb) { listeners.add(cb); return () => { listeners.delete(cb); }; },
  };
  const tick = () => { const c = delivered(elapsedS()); if (c > 0 && c < TOTAL) listeners.forEach((cb) => cb({ streams: ["w-a1"], snapshot_needed: false })); };
  return { transport, tick };
}

export function install() {
  const t0 = Date.now();
  const { transport, tick } = fixtureTransport(() => (Date.now() - t0) / 1000);
  setTransport(transport);
  setInterval(tick, 500);
  location.hash = "#/crew?slug=camp&id=r1";
  // Open the worker's Stream tab, the way a person following a run does.
  const open = setInterval(() => {
    const item = [...document.querySelectorAll<HTMLElement>("[role=treeitem]")].find((e) => e.textContent?.includes("Backend"));
    if (item) { item.click(); clearInterval(open); }
  }, 500);
}
