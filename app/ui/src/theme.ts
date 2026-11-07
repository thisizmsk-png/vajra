// Follows the OS appearance (prefers-color-scheme). Dark is the fallback: no matchMedia, or no light preference.
// The values live in theme/tokens.css; this only picks which set is active via data-theme on the root element.
export const SYSTEM_QUERY = "(prefers-color-scheme: light)";

type Mq = { matches: boolean; addEventListener: (t: "change", cb: () => void) => void; removeEventListener: (t: "change", cb: () => void) => void };
const queryList = (): Mq | null => (typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(SYSTEM_QUERY) : null);

/** The theme the OS asks for right now. */
export function systemTheme(mq: Pick<Mq, "matches"> | null = queryList()): "light" | "dark" {
  return mq?.matches ? "light" : "dark";
}

/** Sets data-theme from the OS and keeps it in step when the OS changes. Returns the stop function. */
export function followSystemTheme(root: HTMLElement = document.documentElement, mq: Mq | null = queryList()): () => void {
  const apply = () => {
    root.dataset.theme = systemTheme(mq);
  };
  apply();
  if (!mq) return () => {};
  mq.addEventListener("change", apply);
  return () => mq.removeEventListener("change", apply);
}
