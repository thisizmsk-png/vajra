// The typing-lag page: the real Director-General composer and transcript in a real browser, on a scripted core where
// 5 agents stream. Built only by vite.lag.config.ts; the app bundle never contains it. app/ui/scripts/typing-lag.mjs types into
// the composer and reads Chrome's own keystroke-to-paint timings.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { setTransport } from "../ipc";
import type { Transport } from "../ipc";
import { CurrentProject } from "../projects/current";
import type { Current } from "../projects/current";
import { DirectorGeneral } from "../screens/orch/DirectorGeneral";
import "../../components/ui/base";

const AGENTS = 5;
const LINES_PER_S = 20;
const STREAM = "orchestrator-p_X-g3";
const params = new URLSearchParams(location.search);
// ?idle=1 runs the same page with nothing streaming: the control the report compares against.
const streaming = !params.has("idle");

let n = 40;
const t0 = Date.now();
const text = (i: number) => (i % 2 ? `lead\tstep ${i} of the plan, checking the reports module and its callers` : `tool\tEdit src/api/file${i % 30}.ts`);
const window_ = () => ({ lines: Array.from({ length: n }, (_, i) => ({ seq: i + 1, text: text(i) })), head_seq: n, first_seq: 1, at_start: true, at_end: true, gap: false });

const listeners = new Set<(p: unknown) => void>();
const transport: Transport = {
  async call(cmd) {
    if (cmd === "lead_status") return { available: true, busy: true, name: "Director-General", state: "running", reason: "working", project_id: "p_X", generation: 3, stream: STREAM };
    if (cmd === "stream_window") return window_();
    throw { error: "unknown_command", detail: cmd };
  },
  on(_e, cb) { listeners.add(cb); return () => { listeners.delete(cb); }; },
};
setTransport(transport);

if (streaming) {
  setInterval(() => {
    n += AGENTS;
    listeners.forEach((cb) => cb({ streams: [STREAM, ...Array.from({ length: AGENTS }, (_, i) => `w-a${i}`)], snapshot_needed: false }));
  }, Math.floor(1000 / LINES_PER_S));
}

const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
const cur: Current = { project, campaigns: [], focus: "c1", runId: null, dgRun: "20261106-2140", proposals: [] };

(window as unknown as { __lag: unknown }).__lag = { startedMs: t0, agents: AGENTS, streaming, lines: () => n };
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <CurrentProject.Provider value={cur}><DirectorGeneral /></CurrentProject.Provider>
  </StrictMode>,
);
