import { ACCOUNTS, AGENTS, USAGE } from "./accountsFixture";
import { DEFAULT_PREFERENCES } from "./preferenceContract";
// Dev only: a fixture engine state for screenshots and the click crawl (DESIGN.md "Verification"). main.tsx loads it
// when the dev server runs with `?fixture`; production builds drop the import (import.meta.env.DEV is false there).
// Nothing here reaches a real engine: every command answers from the objects below.
import { setTransport } from "./ipc";
import type { CrewNodeDto, Transport } from "./ipc";
import { terminalFixture } from "./terminal/fixtureShell";
import { setupReplies } from "./setupFixture";

const RUN = "20261106-2140";
const SLUG = `deliver-${RUN}`;

const n = (id: string, kind: string, title: string, status: string, children: CrewNodeDto[] = [], extra: Partial<CrewNodeDto> = {}): CrewNodeDto =>
  ({ id, kind, title, engine_id: null, role: null, status, overlays: [], children, ...extra });
const agent = (id: string, role: string, engine: string, status: string, overlays: string[] = [], provider?: unknown, diff?: unknown) =>
  n(`agent:${id}`, "worker", role, status, [], { role, engine_id: engine, overlays, ...(provider ? { provider } : {}), ...(diff ? { diff } : {}) });
// A worker's change set (ADR-16 16.10): an exact count, and a lower bound when some new files were not counted.
const exactDiff = { files_total: 4, files: ["src/middleware/rateLimit.ts", "src/server.ts", "src/lib/bucket.ts", "tests/rateLimit.test.ts"], added: 142, removed: 9, partial: false, uncounted: 0 };
const partialDiff = { files_total: 6000, files: ["web/generated/a0001.ts"], added: 5000, removed: 0, partial: true, uncounted: 1000 };
// What each worker ran on (the launch receipt's `effective`), for the run cards, the crew list, the Board and the Crew pane.
const onClaude = { account: "claude", label: "Claude Code", family: "claude", model: null, model_name: null, billing: "subscription", requested: null, error: null };
const onGateway = { account: "openrouter", label: "OpenRouter", family: "deepseek", model: "deepseek/deepseek-v3.2", model_name: "DeepSeek V3.2", billing: "gateway", requested: null, error: { kind: "timeout", action: "retrying", attempt: 1, retry_at: "14:05" } };
const onGrok = { account: "openrouter", label: "OpenRouter", family: "grok", model: "x-ai/grok-4", model_name: "Grok 4", billing: "gateway", requested: null, error: null };
const onRouter = { account: "openrouter", label: "OpenRouter", family: "openrouter", model: null, model_name: null, billing: "gateway", requested: null, error: null };
// A core worker's controls, as the core's harness profile builds them (16.7).
const STOP_NOTE = "Stop ends this worker at once; its uncommitted work stays in its worktree.";
const coreOn = { message: { enabled: true, reason: null }, redirect: { enabled: true, reason: null }, stop: { enabled: true, note: STOP_NOTE } };
const CHECK = "This worker checks the work on its own, so it can't take a message.";
const coreCheck = { message: { enabled: false, reason: CHECK }, redirect: { enabled: false, reason: CHECK }, stop: { enabled: true, note: STOP_NOTE } };
const coreDiff = { files_total: 2, files: ["src/middleware/bucketStore.ts", "tests/bucketStore.test.ts"], added: 58, removed: 3, partial: false, uncounted: 0 };
const onHarnessB = { account: "codex", label: "Codex CLI", family: "codex", model: null, model_name: null, billing: "unknown",
  requested: { model: "deepseek/deepseek-v3.2", model_name: "DeepSeek V3.2", label: "OpenRouter" }, error: null };

const crew = n("lead", "lead", "Lead", "in_progress", [
  n(`run:${RUN}`, "run", "Add per-key rate limiting", "in_progress", [
    n("task:t1", "task", "t1 Design the limiter", "done", [agent("architect", "Architect", "arjuna", "done", [], onClaude)]),
    n("task:t2", "task", "t2 Token bucket middleware", "in_progress", [agent("backend", "Backend", "bhima", "in_progress", ["activity"], onClaude, exactDiff)]),
    n("task:t3", "task", "t3 Show limits on the API keys page", "in_progress", [agent("frontend", "Frontend", "nakula", "in_progress", [], onGateway, partialDiff)]),
    n("task:t4", "task", "t4 429 body and Retry-After header", "question", [agent("sepoy-backend-1", "Backend worker 1", "sepoy-backend-1", "question", [], onHarnessB)]),
    n("task:t5", "task", "t5 Check AC-1 to AC-5", "ready", [agent("verifier", "Verifier", "vidura", "ready", [], onGrok)]),
    n("task:t6", "task", "t6 Document the limits", "done", [agent("docs", "Docs writer", "nakula", "done", [], onRouter)]),
    // Workers the core runs itself (ADR-16 16.7): a Claude task worker it can steer, and a check it never steers.
    n("task:t7", "task", "t7 Cache the bucket state", "in_progress", [n(`core:${RUN}.t7.r1`, "round", "round 1", "in_progress", [], { role: "Backend", provider: onClaude, diff: coreDiff, controls: coreOn })]),
    n(`verification:${RUN}`, "verification", "Verification", "in_progress", [n(`core:${RUN}.verify.r1`, "worker", "Verify", "in_progress", [], { role: "Verifier", provider: onClaude, controls: coreCheck })]),
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
  provider_list: () => ({ schema: "vajra.providers/1", providers: [
    { id: "claude", name: "Claude Code", installed: true, path: "/Users/you/.local/bin/claude", version: "2.1.0", tested: "tested", tested_range: "2.x",
      capabilities: { pty_attach: true, transcript: true, inspect: true, message: true, redirect: true, stop: "hook_block", resume: true, vajra_workflow: true },
      limits: [], login: { route: "Uses your Claude Code login", state: "signed_in", checked_by: "cli_status", detail: null }, billing: { source: "login", key_in_app_env: null } },
    { id: "codex", name: "Codex CLI", installed: true, path: "/opt/homebrew/bin/codex", version: "0.170.0", tested: "untested", tested_range: "0.166",
      capabilities: { pty_attach: true, transcript: false, inspect: false, message: false, redirect: false, stop: "terminate", resume: true, vajra_workflow: false },
      limits: [
        { control: "transcript", code: "no_transcript", reason: "Codex does not write a transcript Vajra can read" },
        { control: "inspect", code: "no_inspect", reason: "Codex has no inspect hook" },
        { control: "message", code: "no_message_mid_run", reason: "Codex cannot take a message mid-run" },
        { control: "redirect", code: "no_redirect_mid_run", reason: "Codex cannot be redirected mid-run" },
        { control: "vajra_workflow", code: "no_vajra_workflow", reason: "Codex does not run the Vajra workflow" },
        { control: "stop", code: "stop_ends_process", reason: "Codex has no hand-off: Stop ends its process" },
      ],
      login: { route: "Uses your Codex login", route_code: "uses_codex_login", state: "signed_in", checked_by: "cli_status", detail: null }, billing: { source: "login", key_in_app_env: "OPENAI_API_KEY" } },
  ] }),
  resume_targets: () => ({ targets: {
    [SLUG]: { session_id: "s-2140", run_id: RUN, cwd: "~/code/harbor-api" },
    "deliver-20261105-1610": { session_id: "s-1610", run_id: "20261105-1610", cwd: "~/code/harbor-api" },
    "deliver-20261104-0930": { session_id: "s-0930", run_id: "20261104-0930", cwd: "~/code/harbor-api", provider: "codex" },
    "deliver-20261103-1120": { session_id: "s-1120", run_id: "20261103-1120", cwd: "~/code/harbor-api/" },
    "deliver-20261102-1505": { session_id: "s-1505", run_id: "20261102-1505", cwd: "~/code/harbor-api" },
    "deliver-20261107-1840": { session_id: "s-1840", run_id: "20261107-1840", cwd: "~/code/harbor-web" },
    "deliver-20261106-0915": { session_id: "s-0915", run_id: "20261106-0915", cwd: "~/code/harbor-web" },
  } }),
  stream_window: (args?: Record<string, unknown>) => {
    if (typeof args?.stream === "string" && args.stream.startsWith("orchestrator-")) {
      const dg = [
        "you\tWhere are we on rate limiting?",
        "lead\tThe run is in its first round. Architect finished the design (t1) and the Redis store (t7) passes AC-4.",
        "lead\tBackend is building the token bucket middleware (t2); Frontend is showing limits on the API keys page (t3).",
        "tool\tget_run_overview deliver-20261106-2140",
        "tool\tRead docs/rate-limits.md",
        "lead\tOne thing needs you: Backend worker 1 asked whether the 429 body should name the limit and the reset time.",
        "you\tYes, name both. What should we do after this run?",
        "lead\tThe admin audit log failed on 2 November with 2 of 7 criteria. I would retry it with a narrower ask, on main, with 2 fix rounds. I have proposed that run below; nothing starts until you start it.",
      ];
      return { lines: dg.map((t, i) => ({ seq: i + 1, text: t })), head_seq: dg.length, first_seq: 1, at_start: true, at_end: true, gap: false };
    }
    // The real StreamWindow shape: up to `maxLines` rows starting at `from` (the newest rows when `from` is null). One worker
    // (`w-sepoy-backend-1`) lost line 5, so any window covering it reports `gap` and leaves that row out.
    const lost = String(args?.stream) === "w-sepoy-backend-1" ? 5 : 0;
    const head = stream.length;
    const max = Math.max(1, Number(args?.maxLines ?? 220));
    const from = args?.from === null || args?.from === undefined ? null : Number(args.from);
    const start = from === null ? Math.max(1, head - max + 1) : Math.max(1, from);
    const end = Math.min(head, start + max - 1);
    return { lines: stream.filter((l) => l.seq >= start && l.seq <= end && l.seq !== lost), head_seq: head, first_seq: 1, at_start: start <= 1, at_end: end >= head, gap: lost >= start && lost <= end };
  },
  lead_status: () => ({ available: true, busy: false, name: "Director-General", state: "running", reason: "a run of this project is going", project_id: "p_01JC3H0000000000000000000A", generation: 3, stream: "orchestrator-p_01JC3H0000000000000000000A-g3" }),
  ask_lead: () => ({ ask_id: "01JC3H0000000000000000ASK1", state: "running", stream: "orchestrator-p_01JC3H0000000000000000000A-g3", lead_session_id: "s", project_id: "p_01JC3H0000000000000000000A", generation: 3, name: "Director-General" }),
  cancel_ask: () => ({}),
  new_conversation: () => ({ project_id: "p_01JC3H0000000000000000000A", generation: 4, stream: "orchestrator-p_01JC3H0000000000000000000A-g4" }),
  start_proposal: () => ({ action_id: "a_fixture1", state: "started", run: {} }),
  dismiss_proposal: () => ({ action_id: "a_fixture2", state: "dismissed" }),
  act_approve_plan: () => ({ receipt: null, error: { error: "not_in_fixture", detail: "approve" } }),
  inbox: () => ({ count: 3, items: [
    { id: "proposal:01JC3H0000000000000000PRP1", kind: "run_proposal", origin: "app", source: "from the Director-General", created: "2026-11-06T22:03:00Z", age_s: 120,
      title: "Start a run?", body: "", ask: "Retry the admin audit log: record who changed roles, API keys and billing settings, with the actor, target, old and new values and the time. Show it under Settings > Audit log with a filter by actor and a CSV export. Skip the retention policy for now.",
      base: "main", rounds: 2, project_id: "p_01JC3H0000000000000000000A", project: "~/code/harbor-api", generation: 3, proposal_sha256: "ab".repeat(32), actions: ["start", "dismiss"],
      workers: [
        { slot: "w1", role: "backend", task: "Token bucket middleware", account: "claude", model: null, effort: "high", source: "role_default", why: "Touches auth; keeps the guard and mid-run messages" },
        { slot: "w2", role: "backend", task: "429 body and Retry-After", account: "claude", model: null, effort: "high", source: "role_default", why: "Same code path as the middleware" },
        { slot: "w3", role: "docs", task: "Document the limits", account: "openrouter", model: "deepseek/deepseek-v3.2", effort: null, source: "role_default", why: "Plain writing, low risk" },
        { slot: "w4", role: "tests", task: "Fixtures for the limits", account: "codex", model: null, effort: "medium", source: "dg", why: "Generated test data" },
      ] },
    { id: "p-deploy", kind: "plan", campaign: SLUG, origin: "app", created: "2026-11-06T22:03:00Z", age_s: 120, title: "Deploy the limiter to staging", body: "", actions: ["approve", "reject"], plan_sha256: "9f2c4e7a1b8d03c5e6f7a89b0c1d2e3f405162738495a6b7c8d9e0f1a2b3c4d5", action: "deploy", plan: { target: "staging", commands: ["make deploy ENV=staging"], rollback: ["make rollback ENV=staging"], health_checks: ["curl -f https://staging.example/health"], risk: "low" }, invalid: null },
    { id: "q-t4", kind: "question", campaign: SLUG, origin: "app", created: "2026-11-06T22:01:00Z", age_s: 240, title: "Should the 429 body name the limit and the reset time?", body: "", actions: ["answer"] },
  ] }),
  run_overview: () => ({
    slug: SLUG, run_id: RUN, stale: false,
    timeline: { phases: [{ name: "Plan", state: "done" }, { name: "Execute", state: "current", round: 1, max_rounds: 3 }, { name: "Verify", state: "pending" }], current: "Execute", status: "active" },
    graph: { waves: 3, tasks: [
      { id: `${RUN}.t1`, local: "t1", title: "Design the limiter", owner: "Architect", status: "done", deps: [], wave: 1, provider: onClaude },
      { id: `${RUN}.t2`, local: "t2", title: "Token bucket middleware", owner: "Backend", status: "in_progress", deps: ["t1"], wave: 2, provider: onClaude },
      { id: `${RUN}.t3`, local: "t3", title: "Show limits on the API keys page", owner: "Frontend", status: "in_progress", deps: ["t1"], wave: 2, provider: onGateway },
      { id: `${RUN}.t4`, local: "t4", title: "429 body and Retry-After header", owner: "Backend worker 1", status: "question", deps: ["t2"], wave: 2, provider: onHarnessB },
      { id: `${RUN}.t5`, local: "t5", title: "Check AC-1 to AC-5", owner: "Verifier", status: "ready", deps: ["t2", "t3", "t4"], wave: 3 },
      { id: `${RUN}.t6`, local: "t6", title: "Document the limits", owner: "Backend", status: "pending", deps: ["t2"], wave: 3 },
      { id: `${RUN}.t7`, local: "t7", title: "Redis store adapter", owner: "Backend", status: "done", deps: ["t1"], wave: 2 },
    ] },
  }),
  evidence_list: () => ({ slug: SLUG, stale: false, rows: [
    { task: `${RUN}.t1`, title: "Design the limiter", owner: "Architect", criterion: "AC-2", status: "pass", head: "4e1c9a2", path: null, commands: 1, report: null },
    { task: `${RUN}.t7`, title: "Redis store adapter", owner: "Backend", criterion: "AC-4", status: "pass", head: "4e1c9a2", path: null, commands: 2, report: null },
    { task: `${RUN}.t7`, title: "Redis store adapter", owner: "Backend", criterion: "AC-5", status: "not_run", head: null, path: null, commands: null, report: null },
    { task: `${RUN}.t2`, title: "Token bucket middleware", owner: "Backend", criterion: "AC-1", status: "not_run", head: null, path: null, commands: null, report: null },
    { task: `${RUN}.t3`, title: "Show limits on the API keys page", owner: "Frontend", criterion: "AC-3", status: "not_run", head: null, path: null, commands: null, report: null },
  ] }),
  active_runs: () => ({ count: 1, runs: [{ run_id: RUN, campaign: SLUG }] }),
  crew: () => crew,
  // Closing finished workers: the Architect has work only its worktree holds; the Docs writer was closed on its own.
  closeout_status: () => ({ workers: [
    { worker: "architect", state: "unsaved_work", changed_files: 3, unpushed_commits: 2, no_upstream: false },
    { worker: "docs", state: "closed" },
    { worker: "verifier", state: "terminal_busy", changed_files: 0, unpushed_commits: 0, no_upstream: false, busy_terminals: 2 },
    { worker: "backend", state: "working" },
  ] }),
  worker_disk: () => ({ copies: 3, bytes: 412_000_000 }),
  act_worker_clean_up: () => ({ receipt: { action_id: "a_fixture4", state: "done" }, result: { removed: ["t1-r1"], disk: { copies: 2, bytes: 268_000_000 } } }),
  act_close_finished: () => ({ receipt: { action_id: "a_fixture3", state: "done" }, result: { closed: [], skipped: [{ worker: "architect", label: "t1", reason: "unsaved_work", changed_files: 3, unpushed_commits: 2, no_upstream: false, busy_terminals: 0 }, { worker: "verifier", label: "t5", reason: "terminal_busy", changed_files: 0, unpushed_commits: 0, no_upstream: false, busy_terminals: 2 }] } }),
  worktree_info: (a) => ({ worker: String(a?.worker), worktree: `.claude/worktrees/${String(a?.worker)}`, branch: `vajra/${RUN}/t2`, base: "4e1c9a2" }),
  task: () => ({ task: { title: "Token bucket middleware", owner_agent: "bhima", deps: ["t1"], files: ["src/middleware/rateLimit.ts", "src/server.ts"], acceptance: ["AC-1", "AC-3"], attempts: 1 } }),
  start_run: () => ({ stream: "s-new", session_id: "u-new", run_id: RUN, campaign: SLUG }),
  read_report: (a) => ({ slug: String(a?.slug), integrity: "ok", text: REPORT }),
  verify_campaign: (a) => ({ slug: String(a?.slug), ok: true, restore: null }),
  kb_search: () => ({ code: 0, hits: [
    { id: "k1", kind: "decision", title: "Rate limits are per API key, not per IP, so shared NATs are not throttled together" },
    { id: "k2", kind: "lesson", title: "Redis MULTI in tests needs the ioredis-mock pipeline shim" },
  ] }),
  diff: () => ({ code: 0, diff, truncated: false, timed_out: false, untracked: ["test/rateLimit.redis.test.ts"] }),
};

/** `mode`: "" is a busy day; "empty" is a first run (no campaigns, no run); "blocked" is a first run whose checks stop a start; "down" is a core that does not answer. */
export function installFixture(mode = "") {
  const empty: typeof replies = { list_campaigns: () => ({ campaigns: [] }), resume_targets: () => ({ targets: {} }), active_runs: () => ({ count: 0, runs: [] }), inbox: () => ({ count: 0, items: [] }) };
  const blocked: typeof replies = { ...empty, preflight: () => ({ rows: [
    { name: "Git", ok: true, detail: "2.46.0", fix: "" },
    { name: "Vajra engine", ok: false, detail: "not installed", fix: "bash scripts/install.sh" },
  ], start_enabled: false }) };
  let revision = 0;
  let global = { ...DEFAULT_PREFERENCES };
  const projects = new Map<string, Record<string, unknown>>();
  const receipts = new Map<string, unknown>();
  // Multi-provider fixture state. A saved key flips the account to "key saved"; the key itself is dropped on the floor.
  let accounts: { schema: string; accounts: Array<Record<string, unknown> & { id: string; credential: object }> } = structuredClone(ACCOUNTS);
  let agents: unknown = structuredClone(AGENTS);
  const setKey = (id: string, saved: boolean) => { accounts = { ...accounts, accounts: accounts.accounts.map((a) => a.id === id ? { ...a, state: saved ? "key_saved" : "key_missing", credential: { ...a.credential, saved } } : a) }; };
  const preferenceView = (id: string | null) => {
    const overrides = id ? projects.get(id) ?? {} : {};
    return { schema: "vajra.preferences/1", rev: revision, project_id: id, global, overrides, caps: { active_runs: 1, workers_per_run: 16, editable: false },
      effective: { ...global, ...overrides }, sources: Object.fromEntries(Object.keys(global).map(k => [k, k in overrides ? "project" : "global"])) };
  };
  const t: Transport = {
    async call(cmd, args) {
      // A core that does not answer: Tauri rejects with a plain string, not the core's {error, detail}.
      if (mode === "down") throw `command ${cmd} failed: the core is not running`;
      if (cmd === "preferences_read") return preferenceView(args?.projectId as string | null);
      if (cmd === "act_preferences_update") {
        const envelope = args?.envelope as { action_id: string; project_id: string; expect_generation: number };
        if (receipts.has(envelope.action_id)) return { receipt: receipts.get(envelope.action_id), replayed: true };
        if (envelope.expect_generation !== revision) return { receipt: { action_id: envelope.action_id, state: "rejected", reason: "revision_conflict" } };
        const id = envelope.project_id === "global" ? null : envelope.project_id;
        const { inherit = [], ...patch } = args?.patch as Record<string, unknown> & { inherit?: string[] };
        if (id) {
          const next = args?.reset ? {} : { ...projects.get(id), ...patch };
          for (const key of inherit) delete next[key];
          projects.set(id, next);
        } else global = args?.reset ? { ...DEFAULT_PREFERENCES } : { ...global, ...patch };
        revision++;
        const receipt = { action_id: envelope.action_id, state: "done" };
        receipts.set(envelope.action_id, receipt);
        return { receipt, result: preferenceView(id) };
      }
      if (cmd === "provider_accounts") return accounts;
      if (cmd === "provider_usage") return USAGE;
      if (cmd === "provider_key_save" || cmd === "provider_key_replace") { setKey(String(args?.accountId), true); return null; }
      if (cmd === "provider_key_remove") { setKey(String(args?.accountId), false); return null; }
      if (cmd === "provider_base_url_save") { accounts = { ...accounts, accounts: accounts.accounts.map((a) => a.id === args?.accountId ? { ...a, base_url: String(args?.baseUrl) } : a) }; return null; }
      if (cmd === "provider_agents") return agents;
      if (cmd === "provider_agents_save") { agents = { schema: AGENTS.schema, ...(args?.agents as object) }; return agents; }
      const r = (mode === "empty" ? empty[cmd] : mode === "blocked" ? blocked[cmd] : undefined) ?? replies[cmd] ?? setupReplies[cmd];
      if (!r) throw { error: "not_in_fixture", detail: cmd };
      return r(args);
    },
    on: () => () => {},
  };
  setTransport(terminalFixture(t));
}
