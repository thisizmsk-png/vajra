// Dev only (loaded through devFixture.ts behind `?fixture=welcome-<state>`): a Mac with no projects, for screenshots of the
// first-run welcome. States: clean, none, failed, signedout, engine, ready. Nothing here reaches a real engine.
const P = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const SITE = { schema: "vajra.project/1", id: P, name: "vajra-site", root: "/Users/you/code/vajra-site", created: null, archived: null, rev: 1 };
const REPOS = [
  { root: "/Users/you/code/vajra-site", name: "vajra-site", worktrees: 0 },
  { root: "/Users/you/code/billing-api", name: "billing-api", worktrees: 2 },
  { root: "/Users/you/dotfiles", name: "dotfiles", worktrees: 0 },
  { root: "/Users/you/Developer/notes-app", name: "notes-app", worktrees: 1 },
];
const row = (id: string, name: string, state: string, version: string | null, extra: object = {}) =>
  ({ id, name, state, version, path: state === "installed" ? `/usr/local/bin/${id}` : null, reason_code: state === "installed" ? null : "not_installed", commands: [], docs_url: "https://example.com/docs", can_sign_in: false, one_click: false, ...extra });

export function welcomeFixture(state: string) {
  let added = false;
  const early = state === "clean" || state === "none" || state === "failed";
  const claudeIn = !early;
  const login = state === "signedout" ? "signed_out" : claudeIn ? "signed_in" : "not_installed";
  return (cmd: string, args?: Record<string, unknown>): unknown | undefined => {
    switch (cmd) {
      case "project_list": return { projects: added ? [SITE] : [], skipped: 0, truncated: false };
      case "project_discover":
        if (state === "failed") throw { error: "core_unavailable", detail: "The app's core didn't answer in time." };
        return { found: state === "none" ? [] : REPOS, truncated: false };
      case "project_add": added = true; return { outcome: "created", project: { ...SITE, root: String(args?.path ?? SITE.root) } };
      case "preflight":
        return { rows: [{ name: "git", ok: !early, detail: early ? "not found" : "2.46.0", fix: "", ...(early ? { action: "install_dev_tools", action_label: "Install developer tools", action_note: "" } : {}) }, { name: "python3", ok: !early, detail: early ? "not found" : "3.12.4", fix: "" }], start_enabled: !early };
      case "setup_list":
        return {
          schema: "vajra.harness_setup/1",
          rows: [row("claude", "Claude Code", claudeIn ? "installed" : "manual", claudeIn ? "2.1.323" : null, { can_sign_in: claudeIn }), row("codex", "Codex CLI", "manual", null), row("gemini", "Gemini CLI", "manual", null)],
          engine: { installed: state === "ready", command: "bash ~/.claude/skills/vajra/scripts/install.sh" },
        };
      case "provider_list":
        return { schema: "vajra.providers/1", providers: [{ id: "claude", name: "Claude Code", installed: claudeIn, path: claudeIn ? "/usr/local/bin/claude" : null, version: claudeIn ? "2.1.323" : null, tested: "tested", tested_range: "2.x",
          capabilities: { pty_attach: true, transcript: true, inspect: true, message: true, redirect: true, stop: "hook_block", resume: true, vajra_workflow: true }, limits: [],
          login: { route: "Uses your Claude Code login", state: login, checked_by: "cli_status", detail: null }, billing: { source: "login", key_in_app_env: null } }] };
      case "act_start_run": return { receipt: { action_id: "a_fixture", state: "done" }, result: { campaign: "fix-date-header", run_id: "20261114-0900", stream: "s1" } };
      case "plugin:dialog|open": return null;
      default: return undefined;
    }
  };
}
