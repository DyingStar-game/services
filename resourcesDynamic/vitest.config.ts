import path from "path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
    environment: "node",
    setupFiles: ["./tests/setup.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json", "html"],
      exclude: ["node_modules/**", "dist/**", "**/*.config.ts", "tests/**"],
    },
    testTimeout: 10000,
  },
  resolve: {
    alias: {
      "@db": path.resolve(__dirname, "./src/db"),
      "@dbduck": path.resolve(__dirname, "./src/duckdb"),
      "@lib": path.resolve(__dirname, "./src/lib"),
      "@routes": path.resolve(__dirname, "./src/routes"),
      "@schema": path.resolve(__dirname, "./src/schema"),
      "@websocket": path.resolve(__dirname, "./src/websocket"),
      "@builder": path.resolve(__dirname, "./tests/builder"),
      "@mocks": path.resolve(__dirname, "./tests/mocks"),
    },
  },
});
