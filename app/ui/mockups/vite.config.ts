import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Mockup-only build: `npx vite --config mockups/vite.config.ts` from app/ui. Not part of the app bundle.
export default defineConfig({
  root: __dirname,
  plugins: [react()],
  server: { port: 1430, strictPort: true, host: "127.0.0.1" },
  // Project-local and git-ignored (dist/); emptyOutDir only ever clears this folder or an explicit MOCKUP_OUT.
  build: { outDir: process.env.MOCKUP_OUT ?? resolve(__dirname, "dist"), emptyOutDir: true },
});
