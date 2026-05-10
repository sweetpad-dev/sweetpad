import path from "node:path";

import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
    environment: "node",
    include: ["src/**/*.spec.ts"],
    setupFiles: ["./tests/setup.js"],
    alias: {
      vscode: path.resolve(__dirname, "src/__mocks__/vscode.ts"),
    },
  },
  define: {
    GLOBAL_SENTRY_DSN: "null",
    GLOBAL_RELEASE_VERSION: JSON.stringify("test"),
  },
});
