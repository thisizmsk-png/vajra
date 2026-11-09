// Fixture answers for the multi-provider commands, shared by the dev fixture (devFixture.ts) and the tests. No secret is
// in here: an api_key account says `saved: true`, never a key, a prefix or a last four.
import type { Agents } from "./accountsContract";

export const ACCOUNTS = {
  schema: "vajra.provider_accounts/1",
  accounts: [
    { id: "claude", provider: "claude", harness: "claude", label: "Claude Code", state: "signed_in", efforts: ["low", "medium", "high"],
      credential: { source: "cli_login", saved: null }, billing: { kind: "subscription", account_label: "Max plan, s…@gmail.com", override_warning: null }, models: [] },
    { id: "codex", provider: "codex", harness: "codex", label: "Codex CLI", state: "cannot_tell", efforts: ["low", "medium", "high"],
      credential: { source: "cli_login", saved: null }, billing: { kind: "unknown", account_label: null, override_warning: "OPENAI_API_KEY in your Codex config" }, models: [] },
    { id: "openrouter", provider: "openrouter", harness: "codex", label: "OpenRouter", state: "key_saved", efforts: ["low", "medium", "high"], base_url: null,
      credential: { source: "api_key", saved: true }, billing: { kind: "gateway", account_label: "OpenRouter key", override_warning: null },
      models: [
        { id: "deepseek/deepseek-v3.2", family: "deepseek", name: "DeepSeek V3.2", tool_check: "passed" },
        { id: "x-ai/grok-4", family: "grok", name: "Grok 4", tool_check: "unchecked" },
      ] },
  ],
};

export const USAGE = {
  schema: "vajra.provider_usage/1",
  usage: [
    { account: "claude", read_at: "2026-11-09T14:00:00Z", windows: [{ kind: "5h", used_pct: 62, resets_at: "16:40" }, { kind: "weekly", used_pct: 84, resets_at: "Thu 09:00" }] },
    { account: "codex", read_at: "2026-11-09T14:00:00Z", windows: [] },
    { account: "openrouter", read_at: "2026-11-09T14:00:00Z", windows: [{ kind: "credit", used_pct: null, resets_at: null, used_usd: 3.1, limit_usd: 10 }] },
  ],
};

export const AGENTS: Agents & { schema: string } = {
  schema: "vajra.project_agents/1",
  default: { account: "claude", model: null, effort: null },
  roles: {
    docs: { account: "openrouter", model: "deepseek/deepseek-v3.2", effort: null },
    tests: { account: "codex", model: null, effort: "medium" },
  },
  allowed: [
    { account: "claude", model: null },
    { account: "codex", model: null },
    { account: "openrouter", model: "deepseek/deepseek-v3.2" },
  ],
};
