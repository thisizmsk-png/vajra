// One realistic run, shared by all three directions so the comparison is about design, not content.
// Everything here is shaped like real engine state (campaign, tasks, stream events, a pending plan change).
import type { StatusKey } from "../strings";

export const repo = { name: "harbor-api", path: "~/code/harbor-api", base: "main", head: "4e1c9a2" };

export const ask = "Add per-key rate limiting to the public API, with a 429 body and a Retry-After header";

export const run = {
  id: "deliver-20261106-2140",
  title: "Add per-key rate limiting",
  phase: "Execute",
  round: 1,
  maxRounds: 3,
  started: "21:40",
  now: "22:07",
  elapsed: "27 min",
  workers: 4,
  memory: "142 MB",
};

export const phases: Array<{ name: string; state: "done" | "live" | "next" }> = [
  { name: "Understand", state: "done" },
  { name: "Plan", state: "done" },
  { name: "Execute", state: "live" },
  { name: "Verify", state: "next" },
  { name: "Deliver", state: "next" },
];

export type Agent = {
  id: string;
  role: string;
  task: string;
  taskId: string;
  status: StatusKey;
  statusLabel?: string;
  added?: number;
  removed?: number;
  files?: number;
  last: string;
  lastAt: string;
};

export const agents: Agent[] = [
  { id: "lead", role: "Lead", task: "Coordinating round 1", taskId: "", status: "in_progress", statusLabel: "Live", last: "Proposed plan v2", lastAt: "22:04" },
  { id: "architect", role: "Architect", task: "Design the limiter: token bucket per key, Redis-backed", taskId: "t1", status: "done", added: 48, removed: 0, files: 1, last: "Wrote docs/adr/0014-rate-limits.md", lastAt: "21:52" },
  { id: "backend", role: "Backend", task: "Token bucket middleware", taskId: "t2", status: "in_progress", added: 159, removed: 9, files: 4, last: "Editing src/middleware/rateLimit.ts", lastAt: "22:06" },
  { id: "frontend", role: "Frontend", task: "Show limits on the API keys page", taskId: "t3", status: "in_progress", added: 88, removed: 12, files: 3, last: "Running vitest KeyLimits", lastAt: "22:06" },
  { id: "backend-1", role: "Backend worker 1", task: "429 body and Retry-After header", taskId: "t4", status: "in_progress", added: 36, removed: 2, files: 2, last: "Editing src/http/errors.ts", lastAt: "22:05" },
  { id: "verifier", role: "Verifier", task: "Check AC-1 to AC-5", taskId: "t5", status: "ready", statusLabel: "Waits on t2, t4", last: "Not started", lastAt: "" },
  { id: "security", role: "Security", task: "Review key handling in the limiter", taskId: "t6", status: "needs_approval", statusLabel: "Needs your approval", last: "Proposed by Lead", lastAt: "22:04" },
];

export const approval = {
  kind: "Plan change",
  title: "Add a Security review before Verify",
  from: "Lead",
  at: "22:04",
  why: "The design keys Redis buckets by the raw API key. Lead wants Security to check key handling before the Verifier runs.",
  changes: [
    { sign: "+", text: "t6 Security: review key handling in the limiter" },
    { sign: "~", text: "t5 Verifier now waits on t6 as well as t2 and t4" },
  ],
  cost: "Adds one worker. Verify starts after t6 is done.",
  ifApprove: "Security starts t6 now in its own worktree. Verifier starts after t2, t4 and t6.",
  ifReject: "Plan v1 stays. Verifier starts after t2 and t4, and AC-5 is checked without a review.",
};

export type StreamEvent =
  | { kind: "tool"; at: string; tool: "Read" | "Edit" | "Bash" | "Write"; target: string; added?: number; removed?: number; out?: Array<{ text: string; tone?: "pass" | "fail" | "muted" }>; exit?: number; live?: boolean }
  | { kind: "action"; at: string; from: string; text: string; state: string };

export const backendStream: StreamEvent[] = [
  { kind: "tool", at: "22:01:12", tool: "Read", target: "src/middleware/index.ts" },
  { kind: "tool", at: "22:01:15", tool: "Read", target: "src/lib/redis.ts" },
  { kind: "tool", at: "22:01:40", tool: "Edit", target: "src/middleware/rateLimit.ts", added: 64, removed: 0 },
  { kind: "tool", at: "22:02:05", tool: "Edit", target: "src/server.ts", added: 3, removed: 1 },
  {
    kind: "tool", at: "22:02:30", tool: "Bash", target: "npm test -- rateLimit", exit: 1,
    out: [
      { text: "✓ allows 60 requests per minute per key (41 ms)", tone: "pass" },
      { text: "✓ refills one token per second (12 ms)", tone: "pass" },
      { text: "✕ isolates buckets between keys (8 ms)", tone: "fail" },
      { text: "Tests: 1 failed, 2 passed, 3 total", tone: "muted" },
    ],
  },
  { kind: "action", at: "22:03:02", from: "Lead", text: "Key the bucket by key id, not the raw key. Security may review hashing.", state: "Delivered 22:03" },
  { kind: "tool", at: "22:03:40", tool: "Edit", target: "src/middleware/rateLimit.ts", added: 12, removed: 6 },
  {
    kind: "tool", at: "22:04:18", tool: "Bash", target: "npm test -- rateLimit", exit: 0,
    out: [{ text: "Tests: 3 passed, 3 total", tone: "pass" }],
  },
  { kind: "tool", at: "22:05:51", tool: "Edit", target: "src/middleware/rateLimit.ts", added: 41, removed: 2 },
  { kind: "tool", at: "22:06:58", tool: "Write", target: "test/rateLimit.redis.test.ts", live: true },
];

// Short tails for the other live agents (direction B shows them as lanes).
export const tails: Record<string, Array<{ at: string; text: string; tone?: "pass" | "fail" | "muted" | "live" }>> = {
  lead: [
    { at: "22:03", text: "Message to Backend: key the bucket by key id" },
    { at: "22:04", text: "Proposed plan v2: add t6 Security review" },
    { at: "22:04", text: "Waiting for your approval on plan v2", tone: "live" },
  ],
  frontend: [
    { at: "21:53", text: "Read src/pages/keys/KeysPage.tsx" },
    { at: "21:54", text: "Read src/api/keys.ts" },
    { at: "21:56", text: "Write src/pages/keys/KeyLimits.tsx +61 −0" },
    { at: "22:01", text: "Edit src/pages/keys/KeysPage.tsx +9 −8" },
    { at: "22:02", text: "✓ KeyLimits renders the per-minute limit (14 ms)", tone: "pass" },
    { at: "22:04", text: "Edit src/pages/keys/KeyLimits.tsx +52 −4" },
    { at: "22:05", text: "Edit src/api/keys.ts +11 −0" },
    { at: "22:06", text: "$ npx vitest run KeyLimits" },
    { at: "22:06", text: "running 4 tests", tone: "live" },
  ],
  "backend-1": [
    { at: "21:54", text: "Read docs/adr/0014-rate-limits.md" },
    { at: "21:56", text: "Read test/http/errors.test.ts" },
    { at: "21:58", text: "Edit test/http/errors.test.ts +12 −0" },
    { at: "22:00", text: "$ npm test -- errors" },
    { at: "22:00", text: "✕ 429 includes Retry-After (6 ms)", tone: "fail" },
    { at: "22:03", text: "Read src/http/errors.ts" },
    { at: "22:04", text: "Edit src/http/errors.ts +24 −2" },
    { at: "22:05", text: "✓ 429 includes Retry-After (9 ms)", tone: "pass" },
    { at: "22:05", text: "Editing src/http/errors.ts", tone: "live" },
  ],
};

export const criteria = [
  { id: "AC-1", text: "A key gets 60 requests a minute, then 429", state: "unknown" as StatusKey },
  { id: "AC-2", text: "429 responses carry Retry-After in seconds", state: "unknown" as StatusKey },
  { id: "AC-3", text: "Buckets are isolated per key", state: "unknown" as StatusKey },
  { id: "AC-4", text: "Limits show on the API keys page", state: "unknown" as StatusKey },
  { id: "AC-5", text: "No raw keys stored in Redis", state: "unknown" as StatusKey },
];

export const preflight = [
  { name: "Claude Code", detail: "2.1.0 at /opt/homebrew/bin/claude", status: "pass" as StatusKey, label: "Pass" },
  { name: "vajra doctor", detail: "18 of 18 checks pass", status: "pass" as StatusKey, label: "Pass" },
  { name: "Hooks", detail: "4 registered in settings.json", status: "pass" as StatusKey, label: "Pass" },
  { name: "OS sandbox", detail: "Off. Turn it on for unattended runs.", status: "needs_approval" as StatusKey, label: "Off" },
];

export type PastRun = { id: string; title: string; outcome: "Delivered" | "Stopped" | "Failed"; detail: string; passing: string; when: string; resumable?: boolean };

export const history: PastRun[] = [
  { id: "deliver-20261105-1610", title: "Fix login redirect loop on expired sessions", outcome: "Delivered", detail: "2 rounds", passing: "6 of 6", when: "Yesterday 18:24" },
  { id: "deliver-20261104-0930", title: "Move invoice export to a background job", outcome: "Stopped", detail: "Verify, round 2", passing: "3 of 5", when: "Nov 4, 11:02", resumable: true },
  { id: "deliver-20261103-1120", title: "Upgrade to Node 22 and fix the test runner", outcome: "Delivered", detail: "1 round", passing: "4 of 4", when: "Nov 3, 12:47" },
  { id: "deliver-20261102-1505", title: "Add an audit log for admin actions", outcome: "Failed", detail: "2 tasks failed after 3 rounds", passing: "2 of 7", when: "Nov 2, 17:31" },
  { id: "deliver-20261031-1012", title: "Paginate the webhooks list endpoint", outcome: "Delivered", detail: "1 round", passing: "3 of 3", when: "Oct 31, 10:58" },
  { id: "deliver-20261030-1544", title: "Replace moment with date-fns in billing", outcome: "Delivered", detail: "2 rounds", passing: "5 of 5", when: "Oct 30, 17:20" },
];

export const repos = ["~/code/harbor-api", "~/code/harbor-web", "~/Development/vajra"];

// Timeline segments for direction C's score, in minutes since 21:40 (now = 27).
export type Seg = { from: number; to: number; kind: "read" | "edit" | "test-pass" | "test-fail" | "think" | "msg" };
export const score: Record<string, Seg[]> = {
  lead: [{ from: 0, to: 4, kind: "think" }, { from: 4, to: 7, kind: "edit" }, { from: 12, to: 13, kind: "msg" }, { from: 23, to: 24, kind: "msg" }, { from: 24, to: 25, kind: "edit" }],
  architect: [{ from: 4, to: 7, kind: "read" }, { from: 7, to: 12, kind: "edit" }],
  backend: [{ from: 13, to: 16, kind: "read" }, { from: 16, to: 21, kind: "edit" }, { from: 22, to: 23, kind: "test-fail" }, { from: 23, to: 24, kind: "edit" }, { from: 24, to: 24.6, kind: "test-pass" }, { from: 25, to: 27, kind: "edit" }],
  frontend: [{ from: 13, to: 15, kind: "read" }, { from: 15, to: 25, kind: "edit" }, { from: 26, to: 27, kind: "test-pass" }],
  "backend-1": [{ from: 14, to: 16, kind: "read" }, { from: 16, to: 24, kind: "edit" }, { from: 25, to: 25.6, kind: "test-pass" }, { from: 25.6, to: 27, kind: "edit" }],
  verifier: [],
  security: [],
};

// ---- Graphite v2 additions ----

/** A second thing waiting for the human, so the inbox has a real queue. */
export const question = {
  kind: "Question",
  title: "Which shape for the 429 body?",
  from: "Backend worker 1",
  taskId: "t4",
  at: "22:06",
  body: "The API already returns errors as { error, message }. RFC 9457 problem+json is the standard for this. Which one should the 429 use?",
  options: ["Keep { error, message } like the other errors", "Use problem+json for 429 only", "Move every error to problem+json (new task)"],
};

export const inboxDone = [
  { kind: "Plan", title: "Plan v1: 5 tasks for per-key rate limiting", who: "You approved", at: "21:47" },
  { kind: "Permission", title: "Backend may run npm test in its worktree", who: "You allowed", at: "22:02" },
];

/** What Backend has touched so far: the review pane sums these (159 added, 9 removed, 4 files). */
export const backendFiles = [
  { path: "src/middleware/rateLimit.ts", added: 117, removed: 8, state: "Edited" },
  { path: "test/rateLimit.test.ts", added: 39, removed: 0, state: "New" },
  { path: "src/server.ts", added: 3, removed: 1, state: "Edited" },
  { path: "test/rateLimit.redis.test.ts", added: 0, removed: 0, state: "Writing" },
];

export const backendCommands = [
  { cmd: "npm test -- rateLimit", at: "22:02", exit: 1, summary: "1 failed, 2 passed" },
  { cmd: "npm test -- rateLimit", at: "22:04", exit: 0, summary: "3 passed" },
];

/** Relative times as the sidebar shows them at 22:07. */
export const ago: Record<string, string> = {
  "deliver-20261106-2140": "now",
  "deliver-20261105-1610": "1d",
  "deliver-20261104-0930": "2d",
  "deliver-20261103-1120": "3d",
  "deliver-20261102-1505": "4d",
  "deliver-20261031-1012": "6d",
  "deliver-20261030-1544": "1w",
};

/** Real Vajra crews, offered on a first run instead of fake sample data. */
export const templates = [
  { name: "Deliver a change", cmd: "/vajra", hint: "Plan, build in worktrees, verify, report" },
  { name: "Security audit", cmd: "/vajra fleet security-audit", hint: "Threat model and review, no code changes" },
  { name: "Architecture review", cmd: "/vajra fleet architecture-review", hint: "ADR, votes, a pre-mortem and a ruling" },
  { name: "Incident response", cmd: "/vajra fleet incident-response", hint: "Triage and a reversible mitigation plan" },
];
