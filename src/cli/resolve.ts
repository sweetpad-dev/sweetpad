import { promises as fs } from "node:fs";

import { getSocketsDir } from "../server/paths";

// Sidecar-only lookup — doesn't probe the socket, so a freshly-dead server can
// briefly resolve. The subsequent connect surfaces ECONNREFUSED, and
// `servers list` cleans up.
export type ResolveOutcome = { kind: "ok"; name: string } | { kind: "none" } | { kind: "ambiguous"; matches: string[] };

export async function resolveServerName(input: string): Promise<ResolveOutcome> {
  let entries: string[];
  try {
    entries = await fs.readdir(getSocketsDir());
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code === "ENOENT") return { kind: "none" };
    throw err;
  }
  const names = entries.filter((e) => e.endsWith(".json")).map((e) => e.slice(0, -".json".length));
  if (names.includes(input)) return { kind: "ok", name: input };
  const matches = names.filter((n) => n.startsWith(input));
  if (matches.length === 0) return { kind: "none" };
  if (matches.length === 1) return { kind: "ok", name: matches[0] };
  return { kind: "ambiguous", matches };
}
