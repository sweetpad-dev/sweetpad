import { findXcodeWorkspaceInDirectory, getCurrentXcodeWorkspacePath } from "../../core/build/utils";
import type { ConfigProvider } from "../../core/config/types";
import type { WorkspaceState } from "../../core/state/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { ProtocolError } from "../../protocol/errors";

/**
 * Resolves the active xcworkspace path for a request: explicit override wins,
 * then the persisted state / user config, finally a cwd-walk autodetect. Used
 * by every method that touches xcodebuild — keeps the precedence consistent
 * so flipping it from one call site can never desync the others.
 */
export async function resolveXcworkspace(
  deps: { workspaceRoot: WorkspaceRoot; config: ConfigProvider; state: WorkspaceState },
  override: string | undefined,
): Promise<string> {
  if (override) return override;

  const fromConfigOrState = getCurrentXcodeWorkspacePath({
    config: deps.config,
    state: deps.state,
    cwd: deps.workspaceRoot.getPath(),
  });
  if (fromConfigOrState) return fromConfigOrState;

  const auto = await findXcodeWorkspaceInDirectory(deps.workspaceRoot.getPath());
  if (auto) return auto;

  throw new ProtocolError("WORKSPACE_NOT_DETECTED", "No .xcworkspace or Package.swift found in this workspace", {
    hint: "Pass --xcworkspace=<path>",
  });
}
