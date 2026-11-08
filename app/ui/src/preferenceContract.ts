// Mirror of the core's bounded preferences DTO; this contains no auth, permissions, hooks or environment values.
export type Preferences = {
  theme: "system" | "dark" | "light"; editor: "system" | "code"; shell: "login" | "sh" | "bash" | "zsh";
  notifications: boolean; agent: "claude" | "codex"; model: string; workers: number; rounds: number;
  shortcuts: { palette: string; settings: string };
};
export type PreferenceKey = keyof Preferences;
export type PreferencePatch = Partial<Preferences> & { inherit?: PreferenceKey[] };
export type PreferenceView = { schema: "vajra.preferences/1"; rev: number; project_id: string | null;
  global: Preferences; overrides: Partial<Preferences>; effective: Preferences; sources: Record<PreferenceKey, "global" | "project"> };
export const DEFAULT_PREFERENCES: Preferences = { theme: "system", editor: "system", shell: "login", notifications: true,
  agent: "claude", model: "", workers: 4, rounds: 2, shortcuts: { palette: "Primary+K", settings: "Primary+," } };
export const SHORTCUT_CHOICES = ["Primary+K", "Primary+,", "Primary+Shift+K", "Primary+Shift+P", "Primary+Shift+S", "F6", "F7"];
export const shortcutKeys = (binding: string) => binding.split("+").map(k => k === "Primary" ? "⌘" : k === "Shift" ? "⇧" : k.toUpperCase());
export function matchesShortcut(e: KeyboardEvent, binding: string): boolean {
  if (e.defaultPrevented || e.isComposing || (e.target instanceof Element && e.target.closest(".xterm, .xterm-helper-textarea, [data-terminal-input]"))) return false;
  const keys = binding.split("+"); const primary = keys.includes("Primary");
  return (primary ? e.metaKey !== e.ctrlKey : !e.metaKey && !e.ctrlKey) && !e.altKey && e.shiftKey === keys.includes("Shift") && e.key.toUpperCase() === keys[keys.length - 1].toUpperCase();
}

export function isPreferenceView(input: unknown): input is PreferenceView {
  if (!input || typeof input !== "object") return false;
  const v = input as PreferenceView;
  const valid = (p: Preferences) => p && ["system", "dark", "light"].includes(p.theme)
    && ["system", "code"].includes(p.editor) && ["login", "sh", "bash", "zsh"].includes(p.shell)
    && typeof p.notifications === "boolean" && ["claude", "codex"].includes(p.agent)
    && typeof p.model === "string" && p.model.length <= 80
    && Number.isInteger(p.workers) && p.workers >= 1 && p.workers <= 16
    && Number.isInteger(p.rounds) && p.rounds >= 1 && p.rounds <= 3
    && p.shortcuts && SHORTCUT_CHOICES.includes(p.shortcuts.palette) && SHORTCUT_CHOICES.includes(p.shortcuts.settings)
    && p.shortcuts.palette !== p.shortcuts.settings;
  return v.schema === "vajra.preferences/1" && Number.isSafeInteger(v.rev) && v.rev >= 0
    && (v.project_id === null || typeof v.project_id === "string") && !!v.overrides && typeof v.overrides === "object"
    && valid(v.global) && valid(v.effective) && !!v.sources
    && Object.keys(DEFAULT_PREFERENCES).every(k => ["global", "project"].includes(v.sources[k as PreferenceKey]));
}
