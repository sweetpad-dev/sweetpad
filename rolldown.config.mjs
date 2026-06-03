import { copyFileSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import path from "node:path";

import { sentryRollupPlugin } from "@sentry/rollup-plugin";
import dotenv from "dotenv";
import { defineConfig } from "rolldown";

import pkg from "./package.json" with { type: "json" };

dotenv.config();

const isProduction = process.env.NODE_ENV === "production";

// The `@sweetpad/lib` native addon lives in the `sweetpad-lib/` workspace and is
// compiled (by `build:sweetpad-lib:*`) into `index.js` + a `.node` binary there.
// It can't be bundled into the JS, so we copy the loader + binary next to the
// extension bundle as `out/lib/` and rewrite the import to require it by path —
// keeping the heavy `sweetpad-lib/` source tree out of the VSIX entirely.
const SWEETPAD_LIB_DIR = path.resolve("sweetpad-lib");
const sweetpadLibPlugin = {
  name: "sweetpad-lib-addon",
  resolveId(source) {
    if (source === "@sweetpad/lib") {
      return { id: "./lib/index.js", external: true };
    }
  },
  writeBundle(outputOptions) {
    const outLibDir = path.join(path.dirname(outputOptions.file), "lib");
    // Recreate from scratch so a stale binary from an earlier build never lingers.
    rmSync(outLibDir, { recursive: true, force: true });
    mkdirSync(outLibDir, { recursive: true });
    const nodeFiles = readdirSync(SWEETPAD_LIB_DIR).filter((f) => f.endsWith(".node"));
    if (nodeFiles.length === 0) {
      this.error("No compiled .node addon found in sweetpad-lib/ — run build:sweetpad-lib:debug first.");
    }
    // The universal binary covers both Mac arches, so ship it alone when present
    // (release build); otherwise ship the single-arch addon (local debug build).
    const universal = nodeFiles.filter((f) => f.includes("universal"));
    const addons = universal.length > 0 ? universal : nodeFiles;
    for (const file of ["index.js", ...addons]) {
      copyFileSync(path.join(SWEETPAD_LIB_DIR, file), path.join(outLibDir, file));
    }
  },
};

const extensionPlugins = [sweetpadLibPlugin];

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
