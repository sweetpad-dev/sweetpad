import * as os from "node:os";
import * as path from "node:path";

import { getStateRoot, getTmpStateRoot, workspaceHash } from "../cli-server/paths";

// Default debug-log file for the BSP server, in the per-workspace tmp state root
// (alongside the build logs) so logs stay out of the project tree.
export function getBspLogPath(workspacePath: string): string {
  return path.join(getTmpStateRoot(workspacePath), "bsp.log");
}

// The BSP server's persisted config (`.sweetpad/bsp.json`): the resolved
// project/scheme/configuration the server reads at startup and watches for
// changes. The extension writes it; it persists across server restarts.
export function getBspConfigFile(workspacePath: string): string {
  return path.join(getStateRoot(workspacePath), "bsp.json");
}

// The BSP server's telemetry socket: a short, stable, project-unique tmpdir path
// (a deep project path under `.sweetpad/` would blow `sun_path`'s ~104-byte cap).
// The extension computes it, writes it into bsp.json for the BSP server to bind,
// and dials it for live logs/status. Stable across restarts, so a relaunched
// extension or server reconnects to the same path.
export function getBspSocketPath(workspacePath: string): string {
  return path.join(os.tmpdir(), `sweetpad-bsp-${workspaceHash(workspacePath)}.sock`);
}
