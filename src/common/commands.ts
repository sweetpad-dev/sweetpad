import * as vscode from "vscode";

import type { BspService } from "../bsp/service";
import type { LspDiagnosticsService } from "../build/lsp-diagnostics";
import type { BuildManager } from "../build/manager";
import type { BuildTreeProvider } from "../build/tree";
import type { CliServerService } from "../cli-server/service";
import type { DestinationsManager } from "../destination/manager";
import type { TunnelManager } from "../devices/tunnel";
import type { SwiftFormattingProvider } from "../format/formatter";
import type { PreviewHostManager } from "../previews/host";
import type { PreviewsManager } from "../previews/manager";
import type { ServeSimManager } from "../simulators/serve-sim";
import type { ProgressStatusBar } from "../system/status-bar";
import type { TestingManager } from "../testing/manager";
import type { ToolsManager } from "../tools/manager";
import { addTreeProviderErrorReporting, errorReporting } from "./error-reporting";
import { type ErrorMessageAction, ExtensionError, TaskError } from "./errors";
import { CommandExecutionScope, type ExecutionScopeService } from "./execution-scope";
import { commonLogger } from "./logger";
import { QuickPickCancelledError } from "./quick-pick";
import type { WorkspaceStateService } from "./workspace-state";

export { BaseExecutionScope, CommandExecutionScope, TaskExecutionScope, type ExecutionScope } from "./execution-scope";

export type {
  LastLaunchedAppContext,
  LastLaunchedAppDeviceContext,
  LastLaunchedAppMacOSContext,
  LastLaunchedAppSimulatorContext,
} from "./workspace-state";

/**
 * Plain dependency bag passed to command handlers, watchers, status bars, and other
 * orchestration code that needs broad access to managers and services. Construct once
 * in `activate()`; everything else just reads fields off it.
 */
export type AppDeps = {
  buildManager: BuildManager;
  testingManager: TestingManager;
  destinationsManager: DestinationsManager;
  toolsManager: ToolsManager;
  tunnelManager: TunnelManager;
  workspaceState: WorkspaceStateService;
  execution: ExecutionScopeService;
  progressStatusBar: ProgressStatusBar;
  formatter: SwiftFormattingProvider;
  vscodeContext: vscode.ExtensionContext;
  buildTreeProvider: BuildTreeProvider;
  lspDiagnostics: LspDiagnosticsService;
  serverService: CliServerService;
  bspService: BspService;
  serveSimManager: ServeSimManager;
  previewsManager: PreviewsManager;
  previewHostManager: PreviewHostManager;
};

/**
 * Register a VS Code command with sweetpad's error reporting, scope, and error UI.
 * `Args` is inferred from the callback signature, so each handler's specific args
 * (e.g. `(deps, item?: BuildTreeItem)`) are type-checked at the call site.
 */
export function registerCommand<Args extends unknown[]>(
  deps: AppDeps,
  commandName: string,
  callback: (deps: AppDeps, ...args: Args) => Promise<unknown>,
): vscode.Disposable {
  return vscode.commands.registerCommand(commandName, async (...args: unknown[]) => {
    const commandContext = new CommandExecutionScope({ commandName: commandName });

    return await errorReporting.withScope(async (scope) => {
      return await deps.execution.startScope(commandContext, async () => {
        scope.setTag("command", commandName);
        try {
          return await callback(deps, ...(args as Args));
        } catch (error) {
          // User can cancel the quick pick dialog by pressing Escape or clicking outside of it.
          // In this case, we just stop the execution of the command and throw a QuickPickCancelledError.
          // Since it is more user action, then an error, we skip the error reporting.
          if (error instanceof QuickPickCancelledError) {
            return;
          }

          if (error instanceof ExtensionError) {
            commonLogger.error(error.message, {
              command: commandName,
              errorContext: error.options?.context,
              error: error,
            });
            if (error instanceof TaskError) {
              return; // do nothing
            }

            await showCommandErrorMessage(`SweetPad: ${error.message}`, {
              actions: error.options?.actions,
            });
            return;
          }

          // Handle unexpected error
          const errorMessage: string =
            error instanceof Error ? error.message : (error?.toString() ?? "[unknown error]");
          commonLogger.error(errorMessage, {
            command: commandName,
            error: error,
          });
          errorReporting.captureException(error);
          await showCommandErrorMessage(`SweetPad: ${errorMessage}`);
        }
      });
    });
  });
}

/**
 * Register a tree data provider with error reporting wrapping.
 */
export function registerTreeDataProvider<T extends vscode.TreeItem>(
  id: string,
  tree: vscode.TreeDataProvider<T>,
): vscode.Disposable {
  return vscode.window.registerTreeDataProvider(id, addTreeProviderErrorReporting(tree));
}

/**
 * Show an error message with optional actions (defaults to "Show logs" + "Close").
 */
export async function showCommandErrorMessage(
  message: string,
  options?: { actions?: ErrorMessageAction[] },
): Promise<void> {
  const closeAction: ErrorMessageAction = {
    label: "Close",
    callback: () => {},
  };
  const showLogsAction: ErrorMessageAction = {
    label: "Show logs",
    callback: () => commonLogger.show(),
  };

  const actions = [closeAction];
  actions.unshift(...(options?.actions ?? [showLogsAction]));

  const actionsLabels = actions.map((action) => action.label);
  const action = await vscode.window.showErrorMessage(message, ...actionsLabels);

  if (action) {
    const callback = actions.find((a) => a.label === action)?.callback;
    if (callback) {
      callback();
    }
  }
}

/** Node.js download/install instructions (shown when no `node` is on PATH). */
export const NODE_DOWNLOAD_URL = "https://nodejs.org/en/download";

/**
 * Warn that no Node.js runtime is on PATH, with a button to the install docs.
 * The BSP server (`bsp-server.js`) launches via `#!/usr/bin/env node`, so
 * without `node` on PATH it can't run. Non-blocking: the caller still
 * completes its work (the warning is informational).
 */
export async function warnNodeRuntimeMissing(subject: string): Promise<void> {
  const install = "Install Node.js";
  const choice = await vscode.window.showWarningMessage(
    `SweetPad: ${subject} needs a Node.js runtime on your PATH, but "node" wasn't found. Install Node.js to use it.`,
    install,
  );
  if (choice === install) {
    void vscode.env.openExternal(vscode.Uri.parse(NODE_DOWNLOAD_URL));
  }
}

/**
 * Wipe all sweetpad.* workspace state and reset coordinated manager state.
 */
export function resetSweetPadState(deps: AppDeps): void {
  deps.workspaceState.reset();
  deps.destinationsManager.setWorkspaceDestinationForBuild(undefined);
  deps.destinationsManager.setWorkspaceDestinationForTesting(undefined);
  deps.buildManager.setDefaultSchemeForBuild(undefined);
  deps.buildManager.setDefaultSchemeForTesting(undefined);
  deps.buildManager.setDefaultConfigurationForBuild(undefined);
  deps.buildManager.setDefaultConfigurationForTesting(undefined);

  void deps.buildManager.refreshSchemes();
  void deps.destinationsManager.refresh();
}
