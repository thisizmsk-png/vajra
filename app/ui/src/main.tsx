/// <reference types="vite/client" />
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { followSystemTheme } from "./theme";

// Before the first render, so the first paint already has the OS theme.
followSystemTheme();

const boot = () =>
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );

// Dev server only: `?fixture` answers every command from src/devFixture.ts, for screenshots and the click crawl.
// `?theme=light` picks the theme for the same reason. Production builds drop both branches.
const q = new URLSearchParams(location.search);
if (import.meta.env.DEV && q.has("fixture")) {
  if (q.get("theme") === "light" || q.get("theme") === "dark") document.documentElement.dataset.theme = q.get("theme")!;
  void import("./devFixture").then((m) => {
    m.installFixture();
    boot();
  });
} else {
  boot();
}
