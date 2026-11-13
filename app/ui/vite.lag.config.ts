// A separate, never-shipped build of the typing-lag page (app/ui/lag.html) for the real-browser gate
// (app/ui/scripts/typing-lag.mjs). The app's own build (vite.config.ts) does not include it.
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  build: { outDir: "dist-lag", emptyOutDir: true, rollupOptions: { input: "lag.html" }, assetsInlineLimit: 0 },
});
