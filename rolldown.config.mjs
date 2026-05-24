import { lstatSync } from "node:fs";
import { join } from "node:path";

import { sentryRollupPlugin } from "@sentry/rollup-plugin";
import dotenv from "dotenv";
import { defineConfig } from "rolldown";

import pkg from "./package.json" with { type: "json" };

dotenv.config();

const isProduction = process.env.NODE_ENV === "production";

/**
 * Fail fast if the bundled sweetpad-lib binary isn't present at
 * `out/bin/sweetpad-darwin-universal`. Catches the "ran `npm install` but
 * forgot to `npm run fetch-sweetpad` or `npm run link-sweetpad-lib`"
 * mistake before rolldown produces a VSIX that would crash at runtime
 * when `sweetpad.system.useSweetpadLib` is enabled.
 */
function ensureSweetpadBin() {
  return {
    name: "ensure-sweetpad-bin",
    buildStart() {
      const binPath = join(process.cwd(), "out", "bin", "sweetpad-darwin-universal");
      try {
        lstatSync(binPath);
      } catch {
        throw new Error(
          `Missing sweetpad-lib binary at ${binPath}.\n` +
            "Run one of:\n" +
            "  npm run fetch-sweetpad      # downloads the published release\n" +
            "  npm run link-sweetpad-lib   # symlinks a local cargo build",
        );
      }
    },
  };
}

const extensionPlugins = [ensureSweetpadBin()];

if (isProduction) {
  extensionPlugins.push(
    sentryRollupPlugin({
      org: "yevhenii-hyzyla",
      project: "sweetpad",
      release: { name: pkg.version },
      authToken: process.env.SENTRY_AUTH_TOKEN,
    }),
  );
}

export default defineConfig([
  {
    input: "./src/extension.ts",
    output: {
      file: "out/extension.js",
      format: "cjs",
      sourcemap: isProduction ? "hidden" : true,
      minify: isProduction,
    },
    platform: "node",
    external: ["vscode"],
    transform: {
      // Lower ES2024 features (notably `await using`) so the bundle parses on
      // older VS Code/Electron runtimes — V8 only gained the `using` parser in
      // ~12.4 (VS Code 1.99). oxc inlines a tiny `_usingCtx` helper that
      // also polyfills `Symbol.asyncDispose` / `SuppressedError`.
      target: "es2022",
      define: {
        GLOBAL_SENTRY_DSN: JSON.stringify(process.env.SENTRY_DSN ?? null),
        GLOBAL_RELEASE_VERSION: isProduction ? JSON.stringify(pkg.version) : JSON.stringify("dev"),
      },
    },
    plugins: extensionPlugins,
  },
  {
    // Bundled CLI client that ships next to the extension. The
    // `sweetpad.system.installCli` command symlinks this file to a directory
    // on the user's PATH.
    input: "./src/cli/index.ts",
    output: {
      file: "out/cli.js",
      format: "cjs",
      sourcemap: isProduction ? "hidden" : true,
      minify: isProduction,
      banner: "#!/usr/bin/env node",
    },
    platform: "node",
    transform: {
      target: "es2022",
      define: {
        GLOBAL_SENTRY_DSN: JSON.stringify(null),
        GLOBAL_RELEASE_VERSION: JSON.stringify(pkg.version),
      },
    },
  },
]);
