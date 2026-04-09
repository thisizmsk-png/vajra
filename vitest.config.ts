import { defineConfig } from "vitest/config";
import { resolve } from "path";

export default defineConfig({
  test: {
    globals: true,
    include: ["tests/**/*.test.ts"],
    testTimeout: 10_000,
  },
  resolve: {
    alias: {
      "better-sqlite3": resolve(__dirname, "node_modules/better-sqlite3"),
    },
  },
});
