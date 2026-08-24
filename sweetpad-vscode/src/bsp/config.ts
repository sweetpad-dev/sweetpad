import type { BuildManager } from "../build/manager";
import { activateCurrentXcodeWorkspacePath, prepareDerivedDataPath } from "../build/utils";
import { getDeveloperDir } from "../common/cli/scripts";
import type { WorkspaceContextService } from "../common/workspace-context";
import type { WorkspaceStateService } from "../common/workspace-state";
import { type BspResolvedConfig, assembleBspConfig } from "./write";

export type { BspResolvedConfig } from "./write";

/**
 * Resolve the BSP config from the current selection, or `null` when no Xcode
 * workspace is detected. This is what the extension writes to the per-project
 * `bsp.json` for the BSP server to read.
 */
export async function buildBspResolvedConfig(deps: {
  workspaceState: WorkspaceStateService;
  workspaceContext: WorkspaceContextService;
  workspacePath: string;
  buildManager: BuildManager;
}): Promise<BspResolvedConfig | null> {
  const xcworkspace = activateCurrentXcodeWorkspacePath({
    workspaceState: deps.workspaceState,
    workspaceContext: deps.workspaceContext,
  });
  if (!xcworkspace) {
    return null;
  }
  return assembleBspConfig({
    workspacePath: deps.workspacePath,
    xcworkspace: xcworkspace,
    developerDir: (await getDeveloperDir({ workspaceRoot: deps.workspacePath })) ?? null,
    scheme: deps.buildManager.getDefaultSchemeForBuild() ?? null,
    configuration: deps.buildManager.getDefaultConfigurationForBuild() ?? "Debug",
    derivedDataPath: prepareDerivedDataPath({ workspaceRoot: deps.workspacePath }) ?? null,
  });
}
