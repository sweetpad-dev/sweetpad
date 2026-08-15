import { cpSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "@vscode/test-cli";

const here = path.dirname(fileURLToPath(import.meta.url));

// The suite opens a real Xcode project so the extension activates the way it
// does for a user (`workspaceContains:**/*.xcodeproj`). It runs against a copy
// of the committed fixture, never the checkout: the tests write
// `buildServer.json` and `.vscode/settings.json` into the workspace root, and a
// build drops artifacts beside it.
const FIXTURE = path.resolve(here, "../sweetpad-lib/fixtures/_synthetic-objc-headers/project");

/**
 * A throwaway copy of the fixture, fresh per run.
 *
 * Carrying build output between runs was tried and measured: it saved about a
 * second and a half, because the suite edits a source file anyway and most of
 * the first build's cost is the extension's one-time device and toolchain
 * discovery rather than compilation. Not worth the state left behind.
 */
function disposableWorkspace() {
  const dir = path.join(mkdtempSync(path.join(tmpdir(), "sweetpad-e2e-")), "project");
  cpSync(FIXTURE, dir, { recursive: true });
  return dir;
}

export default defineConfig({
  label: "e2e",
  files: "out-e2e/**/*.e2e.js",
  workspaceFolder: disposableWorkspace(),
  // `extensionDependencies` names CodeLLDB, and VS Code refuses to activate an
  // extension whose dependency is missing — so the test instance needs it too.
  installExtensions: ["vadimcn.vscode-lldb"],
  mocha: {
    // `suite`/`test`, the interface the VS Code extension samples use.
    ui: "tdd",
    color: true,
    // Xcode-backed cases shell out to `xcodebuild`, which is slow on a cold
    // module cache and slower still on a CI runner.
    timeout: 240_000,
  },
});
