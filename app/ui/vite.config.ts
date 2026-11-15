import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // assetsInlineLimit 0: the app's CSP (default-src 'self') blocks data: images, so a small asset must ship as a same-origin file, never as a data: URI.
  build: { outDir: "dist", emptyOutDir: true, modulePreload: { polyfill: false }, assetsInlineLimit: 0 },
  test: { environment: "jsdom", include: ["tests/**/*.test.{ts,tsx}"], setupFiles: ["tests/setup.ts"], css: false },
});
