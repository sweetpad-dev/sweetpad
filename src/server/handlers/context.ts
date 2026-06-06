import type * as vscode from "vscode";

import type { BuildManager } from "../../build/manager";
import type { WorkspaceStateService } from "../../common/workspace-state";
import type { DestinationsManager } from "../../destination/manager";
import type { BspBridge } from "../bsp-bridge";
import type { BuildSessionRegistry } from "../builds";

export type RpcContext = {
  workspacePath: string;
  extensionVersion: string;
  workspace: WorkspaceStateService;
  buildManager: BuildManager;
  destinationsManager: DestinationsManager;
  buildRegistry: BuildSessionRegistry;
  vscodeContext: vscode.ExtensionContext;
  // sweetpad.* keys from the manifest, prefix-stripped — read by vscodeSettings.list.
  configKeys: string[];
  // Bridges BSP control-channel notifications to VS Code UI; receives setLogLevel.
  bspBridge: BspBridge;
};

export type HandlerFn<P = unknown, R = unknown> = (params: P, ctx: RpcContext) => Promise<R> | R;
