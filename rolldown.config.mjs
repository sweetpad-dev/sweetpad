import { sentryRollupPlugin } from "@sentry/rollup-plugin";
import dotenv from "dotenv";
import { defineConfig } from "rolldown";

import pkg from "./package.json" with { type: "json" };

dotenv.config();

const isProduction = process.env.NODE_ENV === "production";

const plugins = [];

if (isProduction) {
  plugins.push(
    sentryRollupPlugin({
      org: "yevhenii-hyzyla",
      project: "sweetpad",
      release: { name: pkg.version },
      authToken: process.env.SENTRY_AUTH_TOKEN,
    }),
  );
}

const sharedTransform = {
  // rolldown requires `define` here, not at top level
  define: {
    GLOBAL_SENTRY_DSN: JSON.stringify(process.env.SENTRY_DSN ?? null),
    GLOBAL_RELEASE_VERSION: isProduction ? JSON.stringify(pkg.version) : JSON.stringify("dev"),
  },
};

export default defineConfig([
  // VS Code extension. `vscode` is provided by the host at runtime.
  {
    input: "./src/vscode/extension.ts",
    output: {
      file: "out/extension.js",
      format: "cjs",
      sourcemap: isProduction ? "hidden" : true,
      minify: isProduction,
    },
    platform: "node",
    external: ["vscode"],
    transform: sharedTransform,
    plugins,
  },
  // sweetpad-server: standalone Node process. No `vscode` ever.
  {
    input: "./src/server/index.ts",
    output: {
      file: "out/server.js",
      format: "cjs",
      sourcemap: isProduction ? "hidden" : true,
      minify: isProduction,
    },
    platform: "node",
    transform: sharedTransform,
  },
  // sweetpad CLI: the entry the agents invoke. Auto-spawns the server.
  {
    input: "./src/cli/index.ts",
    output: {
      file: "out/cli.js",
      format: "cjs",
      sourcemap: isProduction ? "hidden" : true,
      minify: isProduction,
    },
    platform: "node",
    transform: sharedTransform,
  },
]);
