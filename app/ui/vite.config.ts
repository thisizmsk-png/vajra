import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, modulePreload: { polyfill: false } },
  test: { environment: "jsdom", include: ["tests/**/*.test.{ts,tsx}"], setupFiles: ["tests/setup.ts"], css: false },
});
