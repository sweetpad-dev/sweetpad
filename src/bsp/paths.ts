import * as os from "node:os";
import * as path from "node:path";

import { getProjectStateDir, workspaceHash } from "../cli-server/paths";

// Default debug-log file for the BSP server, in the per-project state dir
// (alongside bsp.json and the build logs) so it stays out of the project tree.
export function getBspLogPath(workspacePath: string): string {
  return path.join(getProjectStateDir(workspacePath), "bsp.log");
}

// The BSP server's persisted config (`bsp.json`): the resolved
// project/scheme/configuration the server reads at startup and watches for
// changes. The extension writes it into the per-project state dir (under the XDG
// state home, out of the project tree) and names it in `buildServer.json`'s
// `argv` via `--config`. It persists across server restarts.
export function getBspConfigFile(workspacePath: string): string {
  return path.join(getProjectStateDir(workspacePath), "bsp.json");
}

// The BSP server's telemetry socket: a short, stable, project-unique tmpdir path
// (a path under the state home could be long enough to blow `sun_path`'s ~104-byte
// cap, so sockets stay in tmpdir). The extension computes it, writes it into
// bsp.json for the BSP server to bind,
// and dials it for live logs/status. Stable across restarts, so a relaunched
// extension or server reconnects to the same path.
export function getBspSocketPath(workspacePath: string): string {
  return path.join(os.tmpdir(), `sweetpad-bsp-${workspaceHash(workspacePath)}.sock`);
}
