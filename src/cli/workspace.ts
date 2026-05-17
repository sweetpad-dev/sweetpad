import * as fs from "node:fs";
import * as path from "node:path";

import { ProtocolError } from "../protocol/errors";

const MARKERS = [".xcworkspace", ".xcodeproj", "Package.swift", "Project.swift"];

/**
 * Walks `start` up the filesystem until it finds a directory containing an
 * `.xcworkspace`, `.xcodeproj`, `Package.swift`, or `Project.swift`. Throws
 * `WORKSPACE_NOT_DETECTED` if it hits `/` without finding one. Mirrors the
 * server-side `CliWorkspaceRoot` so the CLI and server agree on the workspace
 * path used to derive the socket location.
 */
export function resolveWorkspace(start: string): string {
  let current = path.resolve(start);
  while (true) {
    if (hasMarker(current)) return current;
    const parent = path.dirname(current);
    if (parent === current) {
      throw new ProtocolError(
        "WORKSPACE_NOT_DETECTED",
        `No Xcode workspace, project, or Swift package found at or above ${start}`,
      );
    }
    current = parent;
  }
}

function hasMarker(dir: string): boolean {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return false;
  }
  for (const entry of entries) {
    if (MARKERS.some((m) => (m.startsWith(".") ? entry.name.endsWith(m) : entry.name === m))) {
      return true;
    }
  }
  return false;
}
