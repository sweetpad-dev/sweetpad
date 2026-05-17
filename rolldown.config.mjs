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

export default defineConfig({
  input: "./src/vscode/extension.ts",
  output: {
    file: "out/extension.js",
    format: "cjs",
    sourcemap: isProduction ? "hidden" : true,
    minify: isProduction,
  },
  platform: "node",
  external: ["vscode"],
  transform: {
    // rolldown requires `define` here, not at top level
    define: {
      GLOBAL_SENTRY_DSN: JSON.stringify(process.env.SENTRY_DSN ?? null),
      GLOBAL_RELEASE_VERSION: isProduction ? JSON.stringify(pkg.version) : JSON.stringify("dev"),
    },
  },
  plugins,
});
