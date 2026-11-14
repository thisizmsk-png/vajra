// Dev only (loaded through devFixture.ts behind `?fixture`): Settings > Providers > Set up with one-click rows, for
// screenshots of the ADR-17 17b sheet. Shipped catalog entries are all unverified, so the real app never shows these
// rows until a reviewed catalog edit; here Codex and Gemini stand in as verified sources. Nothing installs.
const HASH = "3f".repeat(32);
const SETUP_SCHEMA = "vajra.harness_setup/1";
const PLAN_SCHEMA = "vajra.setup_plan/1";

const LIST = {
  schema: SETUP_SCHEMA,
  rows: [
    { id: "claude", name: "Claude Code", state: "installed", version: "2.1.322", path: "/opt/homebrew/bin/claude", reason_code: null, commands: [], docs_url: "https://code.claude.com/docs/en/setup", can_sign_in: true, one_click: false },
    { id: "codex", name: "Codex CLI", state: "manual", version: null, path: null, reason_code: "not_installed", one_click: true, can_sign_in: false,
      commands: [{ tool: "npm", command: "npm install -g @openai/codex@0.167.1" }, { tool: "brew", command: "brew install --cask codex" }], docs_url: "https://github.com/openai/codex" },
    { id: "gemini", name: "Gemini CLI", state: "broken", version: "0.67.0", path: "/opt/homebrew/bin/gemini", reason_code: "closure_mismatch", one_click: true, can_sign_in: false,
      commands: [{ tool: "brew", command: "brew install gemini-cli" }], docs_url: "https://github.com/google-gemini/gemini-cli" },
  ],
  // D7: the engine stands in as verified too, with an older copy installed, so its update sheet can be screenshot.
  engine: { installed: true, command: "bash ~/.claude/skills/vajra/scripts/install.sh", state: "installed", one_click: true },
};

const ENGINE_HOME = "/Users/you";
const ENGINE_SKILL = `${ENGINE_HOME}/.claude/skills/vajra`;

const NODE = "/opt/homebrew/bin/node";
const NPM = "/opt/homebrew/lib/node_modules/npm/bin/npm-cli.js";
const BREW = "/opt/homebrew/bin/brew";

const PLANS: Record<string, unknown> = {
  codex: {
    schema: PLAN_SCHEMA, id: "codex",
    plan: {
      id: "codex", method: "npm",
      command: [NODE, NPM, "install", "-g", "--ignore-scripts", "--registry=https://registry.npmjs.org/", "<verified tarball>"],
      tools: [{ role: "node", path: NODE, label: null, owned_by_you: true }, { role: "npm", path: NPM, label: null, owned_by_you: true }],
      source: "registry.npmjs.org", version: "0.167.1", target: "/opt/homebrew", pin: { kind: "npm", tgz_sha512: "sha512-…" },
      needs_admin: null, notes: ["npmrc_applies", "owned_by_you"], scope_registry: "https://registry.npmjs.org/", deps: [], hash: HASH,
    },
  },
  gemini: {
    schema: PLAN_SCHEMA, id: "gemini",
    plan: {
      id: "gemini", method: "brew_formula",
      command: [BREW, "install", "homebrew/core/gemini-cli"],
      tools: [{ role: "brew", path: BREW, label: null, owned_by_you: true }],
      source: "homebrew/core", version: "0.68.0", target: "/opt/homebrew", pin: { kind: "brew", token: "homebrew/core/gemini-cli", sha256: "…", url: "…" },
      needs_admin: null, notes: ["brew_not_pinned", "brew_env_applies", "owned_by_you"], scope_registry: null, deps: ["node"], hash: HASH,
    },
  },
  engine: {
    schema: PLAN_SCHEMA, id: "engine",
    plan: {
      id: "engine", method: "engine_bundle",
      command: ["/bin/bash", `${ENGINE_SKILL}/scripts/install.sh`, "--no-hooks"],
      tools: [], source: "Bundled with Vajra Desktop", version: "0.6.3", target: ENGINE_SKILL,
      pin: { kind: "engine", tgz_sha256: "9c".repeat(32), build: "release", case: "update", old_version: "0.6.2", old_root: [16777230, 4211337], workflows: ["vajra-deliver.js", "vajra-feature-build.js"], home: ENGINE_HOME, differs: 7 },
      needs_admin: null, notes: ["no_hooks", "copies_workflows"], scope_registry: null, deps: [], hash: HASH,
    },
  },
};

const OUTPUT: Record<string, string[]> = {
  "setup-codex": ["", "added 1 package in 2s", "", "1 package is looking for funding", "  run `npm fund` for details"],
  "setup-engine": ["Vajra installer", "  scripts and hooks: in place", "  workflows: 2 copied", "  manifest signed", "Skipped Claude Code settings (--no-hooks)."],
  "setup-gemini": ["==> Fetching downloads for: gemini-cli", "==> Pouring gemini-cli--0.68.0.arm64_sequoia.bottle.tar.gz", "🍺  /opt/homebrew/Cellar/gemini-cli/0.68.0: 1,204 files, 41.2MB"],
};

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const setupReplies: Record<string, (a?: Record<string, unknown>) => unknown> = {
  setup_list: () => LIST,
  setup_plan: async (a) => { await wait(600); return PLANS[String(a?.id)] ?? { schema: PLAN_SCHEMA, id: a?.id, manual: "source_not_verified" }; },
  setup_consent: () => ({ token: "0".repeat(32) }),
  setup_run: async (a) => {
    await wait(1500);
    const id = String(a?.id);
    if (id === "engine") return { stream: "setup-engine", report: { outcome: "installed", reason: null, exit: 0, version: "0.6.3" }, case: "update", backup: null };
    return { stream: `setup-${id}`, report: id === "gemini" ? { outcome: "installed", reason: null, exit: 0, version: "0.68.0" } : { outcome: "installed", reason: null, exit: 0, version: "0.167.1" } };
  },
  tail: (a) => {
    const lines = OUTPUT[String(a?.stream)];
    if (!lines) throw { error: "not_in_fixture", detail: "tail" };
    const after = Number(a?.afterSeq ?? 0);
    const all = lines.map((text, i) => ({ seq: i + 1, text }));
    return { lines: all.filter((l) => l.seq > after), gap: false, head_seq: all.length, spill_start: null, spill_end: null };
  },
};
