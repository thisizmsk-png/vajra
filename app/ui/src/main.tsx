/// <reference types="vite/client" />
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { systemTheme } from "./theme";

// Before the first render, so the first paint already has the OS theme.
document.documentElement.dataset.theme = systemTheme();

async function boot() {
  // Perf CI only: the build sets VITE_PERF_FIXTURE, otherwise this branch and the fixture are dropped from the bundle.
  if (import.meta.env.VITE_PERF_FIXTURE) (await import("./perf/fixture")).install();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

// Dev server only: `?fixture` answers every command from src/devFixture.ts, for screenshots and the click crawl.
// `?theme=light` picks the theme and `?palette` opens the ⌘K palette, for the same reason. Production builds drop all three.
const q = new URLSearchParams(location.search);
if (import.meta.env.DEV && q.has("fixture")) {
  if (q.get("theme") === "light" || q.get("theme") === "dark") document.documentElement.dataset.theme = q.get("theme")!;
  void import("./devFixture").then(async (m) => {
    m.installFixture(q.get("fixture") ?? "");
    await boot();
    if (q.has("palette")) setTimeout(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true })), 800);
  });
} else {
  void boot();
}
