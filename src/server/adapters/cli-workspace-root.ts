import * as fs from "node:fs";
import * as path from "node:path";

import { createDirectory, getRelativePath } from "../../core/files";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { ProtocolError } from "../../protocol/errors";

/**
 * Markers that identify a sweetpad-supported workspace. Matches the extension's
 * activation triggers in `package.json`. `Podfile` is omitted intentionally —
 * a CocoaPods-only directory without an .xcworkspace yet isn't buildable.
 */
const WORKSPACE_MARKERS = [".xcworkspace", ".xcodeproj", "Package.swift", "Project.swift"];

/**
 * CLI-side `WorkspaceRoot`. Resolves the project root by walking `cwd` up the
 * filesystem until it finds a sweetpad-recognised marker (`.xcworkspace`,
 * `.xcodeproj`, `Package.swift`, or `Project.swift`). Caches the result for
 * the process lifetime — the workspace doesn't move during a single CLI
 * invocation or server lifetime.
 */
export class CliWorkspaceRoot implements WorkspaceRoot {
  private resolvedPath: string | undefined;

  constructor(private readonly cwd: string) {}

  getPath(): string {
    if (this.resolvedPath) return this.resolvedPath;

    let current = path.resolve(this.cwd);
    while (true) {
      if (hasWorkspaceMarker(current)) {
        this.resolvedPath = current;
        return current;
      }
      const parent = path.dirname(current);
      if (parent === current) {
        throw new ProtocolError(
          "WORKSPACE_NOT_DETECTED",
          `No Xcode workspace, project, or Swift package found at or above ${this.cwd}`,
        );
      }
      current = parent;
    }
  }

  async getStoragePath(): Promise<string> {
    const storagePath = path.join(this.getPath(), ".sweetpad", "storage");
    await createDirectory(storagePath);
    return storagePath;
  }

  getRelativePath(filePath: string): string {
    return getRelativePath(this.getPath(), filePath);
  }
}

function hasWorkspaceMarker(dir: string): boolean {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return false;
  }
  for (const entry of entries) {
    const name = entry.name;
    if (WORKSPACE_MARKERS.some((m) => (m.startsWith(".") ? name.endsWith(m) : name === m))) {
      return true;
    }
  }
  return false;
}
