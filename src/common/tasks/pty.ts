import { createRequire } from "node:module";
import * as path from "node:path";
import type * as NodePty from "node-pty";
import * as vscode from "vscode";
import { commonLogger } from "../logger";

// Reference: vscode-swift uses the same pattern.
// https://github.com/swiftlang/vscode-swift/blob/main/src/utilities/native.ts
// We borrow VS Code's bundled node-pty so we don't have to ship our own
// prebuilt binaries for every platform + Electron ABI combination.

let cached: typeof NodePty | null | undefined;

export function loadNodePty(): typeof NodePty | null {
  if (cached !== undefined) {
    return cached;
  }

  const appRoot = vscode.env.appRoot;
  const candidates = [
    path.join(appRoot, "node_modules.asar", "node-pty"),
    path.join(appRoot, "node_modules", "node-pty"),
  ];

  const req = createRequire(__filename);
  for (const candidate of candidates) {
    try {
      cached = req(candidate) as typeof NodePty;
      commonLogger.debug("Loaded node-pty from VS Code app root", { path: candidate });
      return cached;
    } catch (error) {
      commonLogger.debug("node-pty candidate failed to load", {
        path: candidate,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  cached = null;
  commonLogger.warn("node-pty is not available; v3 task runner will fall back to v2");
  return cached;
}
