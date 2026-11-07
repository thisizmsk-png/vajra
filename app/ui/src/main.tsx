import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";

async function boot() {
  // Perf CI only: the build sets VITE_PERF_FIXTURE, otherwise this branch and the fixture are dropped from the bundle.
  if (import.meta.env.VITE_PERF_FIXTURE) (await import("./perf/fixture")).install();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
void boot();
