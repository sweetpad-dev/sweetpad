import { createHash } from "node:crypto";
import * as os from "node:os";
import * as path from "node:path";

import { getStateRoot } from "../server/paths";

// Default debug-log file for the BSP server, under the shared `.sweetpad/`
// runtime-state dir (not a subdir) so it's writable without creating anything.
export function getBspLogPath(workspacePath: string): string {
  return path.join(getStateRoot(workspacePath), "bsp.log");
}

// The BSP server's persisted config (`.sweetpad/bsp.json`): the resolved
// project/scheme/configuration the server reads at startup and watches for
// changes. The extension writes it; it persists across server restarts (unlike
// the ephemeral `run/*.json` connection files).
export function getBspConfigFile(workspacePath: string): string {
  return path.join(getStateRoot(workspacePath), "bsp.json");
}

// The BSP server's telemetry socket: a short, stable, project-unique tmpdir path
// (a deep project path under `.sweetpad/` would blow `sun_path`'s ~104-byte cap).
// The extension computes it, writes it into bsp.json for the BSP server to bind,
// and dials it for live logs/status. Stable across restarts, so a relaunched
// extension or server reconnects to the same path.
export function getBspSocketPath(workspacePath: string): string {
  const hash = createHash("sha1").update(workspacePath).digest("hex").slice(0, 12);
  return path.join(os.tmpdir(), `sweetpad-bsp-${hash}.sock`);
}
