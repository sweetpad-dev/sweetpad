import { promises as fs } from "node:fs";
import * as path from "node:path";

import { findProjectRoot, getRunDir } from "../server/paths";
import type { ServerMetadata } from "../server/types";

// Connection-file-only lookup (no socket probe), so a freshly-dead server can
// briefly resolve. The subsequent connect surfaces ECONNREFUSED, and
// `servers list` cleans up. Scoped to the project the CLI is run inside.
export type ResolveOutcome = { kind: "ok"; name: string } | { kind: "none" } | { kind: "ambiguous"; matches: string[] };

// The names of `kind: "extension"` servers in this project's `.sweetpad/run`
// (BSP control endpoints share the directory but aren't CLI targets).
async function extensionServerNames(startDir: string): Promise<string[]> {
  const root = await findProjectRoot(startDir);
  if (!root) return [];
  const dir = getRunDir(root);
  let entries: string[];
  try {
    entries = await fs.readdir(dir);
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code === "ENOENT") return [];
    throw err;
  }
  const names: string[] = [];
  for (const entry of entries) {
    if (!entry.endsWith(".json")) continue;
    try {
      const meta = JSON.parse(await fs.readFile(path.join(dir, entry), "utf8")) as ServerMetadata;
      if (meta.kind === "extension" && typeof meta.name === "string") names.push(meta.name);
    } catch {
      // ignore unreadable/corrupt connection files
    }
  }
  return names;
}

export async function resolveServerName(input: string, startDir: string = process.cwd()): Promise<ResolveOutcome> {
  const names = await extensionServerNames(startDir);
  if (names.includes(input)) return { kind: "ok", name: input };
  const matches = names.filter((n) => n.startsWith(input));
  if (matches.length === 0) return { kind: "none" };
  if (matches.length === 1) return { kind: "ok", name: matches[0] };
  return { kind: "ambiguous", matches };
}
