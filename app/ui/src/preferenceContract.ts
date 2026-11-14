// Mirror of the core's bounded preferences DTO; this contains no auth, permissions, hooks or environment values.
export type Preferences = {
  theme: "system" | "dark" | "light"; editor: "system" | "code"; shell: "login" | "sh" | "bash" | "zsh";
  notifications: boolean; notifications_text: boolean; agent: "claude" | "codex"; model: string; workers: number; rounds: number; auto_close_finished: boolean;
  shortcuts: { palette: string; settings: string };
  /** The ping rule (global only; the core refuses a project override). Absent from an older core: read DEFAULT_PING. */
  ping?: PingRule;
};
/** When the app pings you about a new Needs-you item. It decides only whether a notice shows now, later or not at all. */
export type PingRule = { per_hour: number; batch_minutes: number; decisions: boolean; questions: boolean; blocked: boolean };
export const DEFAULT_PING: PingRule = { per_hour: 6, batch_minutes: 2, decisions: true, questions: true, blocked: true };
export const PING_PER_HOUR = [1, 2, 3, 6, 12, 30] as const;
export const PING_BATCH_MINUTES = [0, 1, 2, 5, 10] as const;
export function isPingRule(r: unknown): r is PingRule {
  const p = r as PingRule;
  return !!p && typeof p === "object" && Number.isInteger(p.per_hour) && p.per_hour >= 1 && p.per_hour <= 30
    && Number.isInteger(p.batch_minutes) && p.batch_minutes >= 0 && p.batch_minutes <= 10
    && typeof p.decisions === "boolean" && typeof p.questions === "boolean" && typeof p.blocked === "boolean";
}
export type PreferenceKey = keyof Preferences;
export type PreferencePatch = Partial<Preferences> & { inherit?: PreferenceKey[] };
export type PreferenceView = { schema: "vajra.preferences/1"; rev: number; project_id: string | null;
  caps: { active_runs: number; workers_per_run: number; editable: false };
  global: Preferences; overrides: Partial<Preferences>; effective: Preferences; sources: Record<PreferenceKey, "global" | "project"> };
export const DEFAULT_PREFERENCES: Preferences = { theme: "system", editor: "system", shell: "login", notifications: true, notifications_text: false,
  agent: "claude", model: "", workers: 4, rounds: 2, auto_close_finished: true, shortcuts: { palette: "Primary+K", settings: "Primary+," }, ping: DEFAULT_PING };
export const SHORTCUT_CHOICES = ["Primary+K", "Primary+,", "Primary+Shift+K", "Primary+Shift+P", "Primary+Shift+S", "F6", "F7"];
export const shortcutKeys = (binding: string) => binding.split("+").map(k => k === "Primary" ? "⌘" : k === "Shift" ? "⇧" : k.toUpperCase());
/** Read once: the platform modifier is Command on macOS and Control elsewhere (the same test Shell uses for the overlay). */
export const IS_MAC = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
/** `Primary` is the platform modifier only. On macOS Control stays the terminal's key, so Control+K never opens the palette. */
export function matchesShortcut(e: KeyboardEvent, binding: string, mac = IS_MAC): boolean {
  if (e.defaultPrevented || e.isComposing || (e.target instanceof Element && e.target.closest(".xterm, .xterm-helper-textarea, [data-terminal-input]"))) return false;
  const keys = binding.split("+"); const primary = keys.includes("Primary");
  const modifier = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  return (primary ? modifier : !e.metaKey && !e.ctrlKey) && !e.altKey && e.shiftKey === keys.includes("Shift") && e.key.toUpperCase() === keys[keys.length - 1].toUpperCase();
}

export function isPreferenceView(input: unknown): input is PreferenceView {
  if (!input || typeof input !== "object") return false;
  const v = input as PreferenceView;
  const valid = (p: Preferences) => p && ["system", "dark", "light"].includes(p.theme)
    && ["system", "code"].includes(p.editor) && ["login", "sh", "bash", "zsh"].includes(p.shell)
    && typeof p.notifications === "boolean" && typeof p.notifications_text === "boolean" && ["claude", "codex"].includes(p.agent)
    && typeof p.model === "string" && p.model.length <= 80
    && Number.isInteger(p.workers) && p.workers >= 1 && p.workers <= 16
    && Number.isInteger(p.rounds) && p.rounds >= 1 && p.rounds <= 3
    && typeof p.auto_close_finished === "boolean"
    && (p.ping === undefined || isPingRule(p.ping))
    && p.shortcuts && SHORTCUT_CHOICES.includes(p.shortcuts.palette) && SHORTCUT_CHOICES.includes(p.shortcuts.settings)
    && p.shortcuts.palette !== p.shortcuts.settings;
  return v.schema === "vajra.preferences/1" && Number.isSafeInteger(v.rev) && v.rev >= 0
    && (v.project_id === null || typeof v.project_id === "string") && !!v.overrides && typeof v.overrides === "object"
    && valid(v.global) && valid(v.effective) && !!v.sources
    && Object.keys(DEFAULT_PREFERENCES).every(k => ["global", "project"].includes(v.sources[k as PreferenceKey]));
}

export type PreferenceEnvelope = { action_id: string; project_id: string; expect_generation: number };
export type PreferenceReceipt = { action_id: string; state: "accepted" | "done" | "rejected" | "failed" | "unknown"; reason?: string | null };
export type PreferenceActResult = { receipt: PreferenceReceipt | null; result?: PreferenceView; error?: { error: string }; replayed?: boolean };
