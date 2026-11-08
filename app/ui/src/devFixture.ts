// Dev only: a fixture engine state for screenshots and the click crawl (DESIGN.md "Verification"). main.tsx loads it
// when the dev server runs with `?fixture`; production builds drop the import (import.meta.env.DEV is false there).
// Nothing here reaches a real engine: every command answers from the objects below.
import { setTransport } from "./ipc";
import type { CrewNodeDto, Transport } from "./ipc";
import { terminalFixture } from "./terminal/fixtureShell";

const RUN = "20261106-2140";
const SLUG = `deliver-${RUN}`;

const n = (id: string, kind: string, title: string, status: string, children: CrewNodeDto[] = [], extra: Partial<CrewNodeDto> = {}): CrewNodeDto =>
  ({ id, kind, title, engine_id: null, role: null, status, overlays: [], children, ...extra });
const agent = (id: string, role: string, engine: string, status: string, overlays: string[] = []) =>
  n(`agent:${id}`, "worker", role, status, [], { role, engine_id: engine, overlays });

const crew = n("lead", "lead", "Lead", "in_progress", [
  n(`run:${RUN}`, "run", "Add per-key rate limiting", "in_progress", [
    n("task:t1", "task", "t1 Design the limiter", "done", [agent("architect", "Architect", "arjuna", "done")]),
    n("task:t2", "task", "t2 Token bucket middleware", "in_progress", [agent("backend", "Backend", "bhima", "in_progress", ["activity"])]),
    n("task:t3", "task", "t3 Show limits on the API keys page", "in_progress", [agent("frontend", "Frontend", "nakula", "in_progress")]),
    n("task:t4", "task", "t4 429 body and Retry-After header", "question", [agent("sepoy-backend-1", "Backend worker 1", "sepoy-backend-1", "question")]),
    n("task:t5", "task", "t5 Check AC-1 to AC-5", "ready", [agent("verifier", "Verifier", "vidura", "ready")]),
  ]),
]);

const stream = [
  "Read src/middleware/index.ts",
  "Read src/lib/redis.ts",
  "Edit src/middleware/rateLimit.ts (+64 -0)",
  "Edit src/server.ts (+3 -1)",
  "$ npm test -- rateLimit",
  "\u001b[32m✓\u001b[0m allows 60 requests per minute per key (41 ms)",
  "\u001b[32m✓\u001b[0m refills one token per second (12 ms)",
  "\u001b[31m✕\u001b[0m isolates buckets between keys (8 ms)",
  "\u001b[2mTests: 1 failed, 2 passed, 3 total\u001b[0m",
  "Message from Lead: key the bucket by key id, not the raw key.",
  "Edit src/middleware/rateLimit.ts (+12 -6)",
  "$ npm test -- rateLimit",
  "\u001b[32mTests: 3 passed, 3 total\u001b[0m",
  "Edit src/middleware/rateLimit.ts (+41 -2)",
  "Write test/rateLimit.redis.test.ts",
].map((text, i) => ({ seq: i + 1, text }));

const diff = `diff --git a/src/server.ts b/src/server.ts
--- a/src/server.ts
+++ b/src/server.ts
@@ -12,7 +12,9 @@ import { errors } from "./http/errors";
 const app = express();
 app.use(json());
-app.use(router);
+app.use(rateLimit({ perMinute: 60, store: redis }));
+app.use(router);
+app.use(errors);

 export { app };
diff --git a/src/middleware/rateLimit.ts b/src/middleware/rateLimit.ts
--- /dev/null
+++ b/src/middleware/rateLimit.ts
@@ -0,0 +1,6 @@
+import type { RequestHandler } from "express";
+import { bucketFor } from "../lib/bucket";
+
+export const rateLimit = (o: { perMinute: number; store: Store }): RequestHandler => async (req, res, next) => {
+  const ok = await bucketFor(o.store, req.apiKeyId, o.perMinute).take();
+  return ok ? next() : res.status(429).set("Retry-After", "1").json({ error: "rate_limited" });
+};
`;

const REPORT = `# Delivery report: Fix login redirect loop on expired sessions

Campaign deliver-20261105-1610, run 20261105-1610. 6 of 6 criteria pass.

## What changed
- src/auth/session.ts: an expired session now clears the cookie before the redirect.
- src/routes/login.ts: the return URL is checked against the allow list once.
- test/auth/redirect.test.ts: 4 new cases, including the loop from the bug report.

## Evidence
AC-1 pass  npm test -- redirect  exit 0
AC-2 pass  npm run lint          exit 0
`;

const replies: Record<string, (a?: Record<string, unknown>) => unknown> = {
  preflight: () => ({
    rows: [
      { name: "Claude Code", ok: true, detail: "2.1.0", fix: "" },
      { name: "Vajra hooks", ok: true, detail: "registered", fix: "" },
      { name: "Git", ok: true, detail: "2.46.0", fix: "" },
      { name: "OS sandbox", ok: false, detail: "off", fix: "bash scripts/enable-sandbox.sh" },
    ],
    start_enabled: true,
  }),
  list_campaigns: () => ({
    campaigns: [
      { slug: SLUG, goal: "Add per-key rate limiting", status: "active", passing: 0, total: 5, updated: "2026-11-06 22:05", integrity: "unknown" },
      { slug: "deliver-20261105-1610", goal: "Fix login redirect loop on expired sessions", status: "done", passing: 6, total: 6, updated: "2026-11-05 17:10", integrity: "ok" },
      { slug: "deliver-20261104-0930", goal: "Move invoice export to a background job", status: "stopped", passing: 3, total: 5, updated: "2026-11-04 10:30", integrity: "ok" },
      { slug: "deliver-20261103-1120", goal: "Upgrade to Node 22 and fix the test runner", status: "done", passing: 4, total: 4, updated: "2026-11-03 12:20", integrity: "ok" },
      { slug: "deliver-20261102-1505", goal: "Add an audit log for admin actions", status: "failed", passing: 2, total: 7, updated: "2026-11-02 16:05", integrity: "failed" },
      { slug: "deliver-20261107-1840", goal: "Ship the usage dashboard", status: "done", passing: 5, total: 5, created: "2026-11-01 08:00", updated: "2026-11-07 19:40", integrity: "ok" },
      { slug: "deliver-20261106-0915", goal: "Stop the flaky checkout test", status: "stopped", passing: 1, total: 3, created: "2026-11-01 08:30", updated: "2026-11-06 09:15", integrity: "ok" },
      { slug: "deliver-20261031-1200", goal: "Rename the billing tables", status: "done", passing: 3, total: 3, updated: "2026-10-31 12:00", integrity: "unknown" },
    ],
  }),
  // Three folders: harbor-api (the live run), harbor-web, and one campaign started in a terminal (Unsorted).
  resume_targets: () => ({ targets: {
    [SLUG]: { session_id: "s-2140", run_id: RUN, cwd: "~/code/harbor-api" },
    "deliver-20261105-1610": { session_id: "s-1610", run_id: "20261105-1610", cwd: "~/code/harbor-api" },
    "deliver-20261104-0930": { session_id: "s-0930", run_id: "20261104-0930", cwd: "~/code/harbor-api" },
    "deliver-20261103-1120": { session_id: "s-1120", run_id: "20261103-1120", cwd: "~/code/harbor-api/" },
    "deliver-20261102-1505": { session_id: "s-1505", run_id: "20261102-1505", cwd: "~/code/harbor-api" },
    "deliver-20261107-1840": { session_id: "s-1840", run_id: "20261107-1840", cwd: "~/code/harbor-web" },
    "deliver-20261106-0915": { session_id: "s-0915", run_id: "20261106-0915", cwd: "~/code/harbor-web" },
  } }),
  stream_window: () => {
    const text = [
      "Reading src/middleware/index.ts", "Adding TokenBucket with capacity and refill per key", "Wrote src/middleware/rateLimit.ts (84 lines)",
      "Running npm test -- rateLimit", "PASS tests/rateLimit.test.ts (6 tests)", "Next: wire the limiter into the router",
    ];
    return { lines: text.map((t, i) => ({ seq: i + 1, text: t })), head_seq: text.length, first_seq: 1, at_start: true, at_end: true, gap: false };
  },
  act_approve_plan: () => ({ receipt: null, error: { error: "not_in_fixture", detail: "approve" } }),
  inbox: () => ({ count: 2, items: [
    { id: "p-deploy", kind: "plan", campaign: SLUG, origin: "app", created: "2026-11-06T22:03:00Z", age_s: 120, title: "Deploy the limiter to staging", body: "", actions: ["approve", "reject"], plan_sha256: "9f2c4e7a1b8d03c5e6f7a89b0c1d2e3f405162738495a6b7c8d9e0f1a2b3c4d5", action: "deploy", plan: { target: "staging", commands: ["make deploy ENV=staging"], rollback: ["make rollback ENV=staging"], health_checks: ["curl -f https://staging.example/health"], risk: "low" }, invalid: null },
    { id: "q-t4", kind: "question", campaign: SLUG, origin: "app", created: "2026-11-06T22:01:00Z", age_s: 240, title: "Should the 429 body name the limit and the reset time?", body: "", actions: ["answer"] },
  ] }),
  run_overview: () => ({
    slug: SLUG, run_id: RUN, stale: false,
    timeline: { phases: [{ name: "Plan", state: "done" }, { name: "Execute", state: "current", round: 1, max_rounds: 3 }, { name: "Verify", state: "pending" }], current: "Execute", status: "active" },
    graph: { waves: 3, tasks: [
      { id: `${RUN}.t1`, local: "t1", title: "Design the limiter", owner: "arjuna", status: "done", deps: [], wave: 1 },
      { id: `${RUN}.t2`, local: "t2", title: "Token bucket middleware", owner: "bhima", status: "in_progress", deps: ["t1"], wave: 2 },
      { id: `${RUN}.t3`, local: "t3", title: "Show limits on the API keys page", owner: "nakula", status: "in_progress", deps: ["t1"], wave: 2 },
      { id: `${RUN}.t4`, local: "t4", title: "429 body and Retry-After header", owner: "sepoy-backend-1", status: "question", deps: ["t2"], wave: 2 },
      { id: `${RUN}.t5`, local: "t5", title: "Check AC-1 to AC-5", owner: "vidura", status: "ready", deps: ["t2", "t3", "t4"], wave: 3 },
      { id: `${RUN}.t6`, local: "t6", title: "Document the limits", owner: "bhima", status: "pending", deps: ["t2"], wave: 3 },
      { id: `${RUN}.t7`, local: "t7", title: "Redis store adapter", owner: "bhima", status: "done", deps: ["t1"], wave: 2 },
    ] },
  }),
  evidence_list: () => ({ slug: SLUG, stale: false, rows: [
    { task: `${RUN}.t1`, title: "Design the limiter", owner: "arjuna", criterion: "AC-2", status: "pass", head: "4e1c9a2", path: null, commands: 1, report: null },
    { task: `${RUN}.t7`, title: "Redis store adapter", owner: "bhima", criterion: "AC-4", status: "pass", head: "4e1c9a2", path: null, commands: 2, report: null },
    { task: `${RUN}.t7`, title: "Redis store adapter", owner: "bhima", criterion: "AC-5", status: "not_run", head: null, path: null, commands: null, report: null },
    { task: `${RUN}.t2`, title: "Token bucket middleware", owner: "bhima", criterion: "AC-1", status: "not_run", head: null, path: null, commands: null, report: null },
    { task: `${RUN}.t3`, title: "Show limits on the API keys page", owner: "nakula", criterion: "AC-3", status: "not_run", head: null, path: null, commands: null, report: null },
  ] }),
  active_runs: () => ({ count: 1, runs: [{ run_id: RUN, campaign: SLUG }] }),
  crew: () => crew,
  worktree_info: (a) => ({ worker: String(a?.worker), worktree: `.claude/worktrees/${String(a?.worker)}`, branch: `vajra/${RUN}/t2`, base: "4e1c9a2" }),
  task: () => ({ task: { title: "Token bucket middleware", owner_agent: "bhima", deps: ["t1"], files: ["src/middleware/rateLimit.ts", "src/server.ts"], acceptance: ["AC-1", "AC-3"], attempts: 1 } }),
  tail: (a) => {
    const after = Number(a?.afterSeq ?? 0);
    return { lines: stream.filter((l) => l.seq > after), gap: false, head_seq: stream.length, spill_start: null, spill_end: null };
  },
  start_run: () => ({ stream: "s-new", session_id: "u-new", run_id: RUN, campaign: SLUG }),
  read_report: (a) => ({ slug: String(a?.slug), integrity: "ok", text: REPORT }),
  verify_campaign: (a) => ({ slug: String(a?.slug), ok: true, restore: null }),
  kb_search: () => ({ code: 0, hits: [
    { id: "k1", kind: "decision", title: "Rate limits are per API key, not per IP, so shared NATs are not throttled together" },
    { id: "k2", kind: "lesson", title: "Redis MULTI in tests needs the ioredis-mock pipeline shim" },
  ] }),
  read_spill_before: () => ({ lines: [], at_start: true, gap: false }),
  diff: () => ({ code: 0, diff, truncated: false, timed_out: false, untracked: ["test/rateLimit.redis.test.ts"] }),
};

/** `mode`: "" is a busy day; "empty" is a first run (no campaigns, no run); "down" is a core that does not answer. */
export function installFixture(mode = "") {
  const empty: typeof replies = { list_campaigns: () => ({ campaigns: [] }), resume_targets: () => ({ targets: {} }), active_runs: () => ({ count: 0, runs: [] }), inbox: () => ({ count: 0, items: [] }) };
  const t: Transport = {
    async call(cmd, args) {
      // A core that does not answer: Tauri rejects with a plain string, not the core's {error, detail}.
      if (mode === "down") throw `command ${cmd} failed: the core is not running`;
      const r = (mode === "empty" ? empty[cmd] : undefined) ?? replies[cmd];
      if (!r) throw { error: "not_in_fixture", detail: cmd };
      return r(args);
    },
    on: () => () => {},
  };
  setTransport(terminalFixture(t));
}
