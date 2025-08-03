import * as events from "node:events";
import * as vscode from "vscode";
import { BuildManager } from "../build/manager";
import { DestinationsManager } from "../destination/manager";
import type { DestinationType, SelectedDestination } from "../destination/types";
import { DevicesManager } from "../devices/manager";
import { SwiftFormattingProvider } from "../format/formatter";
import { SimulatorsManager } from "../simulators/manager";
import { ProgressStatusBar } from "../system/status-bar";
import { TestingManager } from "../testing/manager";
import { ToolsManager } from "../tools/manager";
import { addTreeProviderErrorReporting, errorReporting } from "./error-reporting";
import { type ErrorMessageAction, ExtensionError, TaskError } from "./errors";
import { CommandExecutionScope, type ExecutionScope, ExecutionScopeManager } from "./execution-scope";
import { commonLogger } from "./logger";
import { QuickPickCancelledError } from "./quick-pick";

export type LastLaunchedAppDeviceContext = {
  type: "device";
  appPath: string; // Example: "/Users/username/Library/Developer/Xcode/DerivedData/MyApp-..."
  appName: string; // Example: "MyApp.app"
  destinationId: string; // Example: "00008030-001A0A3E0A68002E"
  destinationType: DestinationType; // Example: "iOS"
};

export type LastLaunchedAppSimulatorContext = {
  type: "simulator";
  appPath: string;
};

export type LastLaunchedAppMacOSContext = {
  type: "macos";
  appPath: string;
};

export type LastLaunchedAppContext =
  | LastLaunchedAppDeviceContext
  | LastLaunchedAppSimulatorContext
  | LastLaunchedAppMacOSContext;

type WorkspaceTypes = {
  _version: undefined | 1;
  "build.xcodeWorkspacePath": string;
  "build.xcodeProjectPath": string;
  "build.xcodeScheme": string;
  "build.xcodeConfiguration": string;
  "build.xcodeDestination": SelectedDestination;
  "build.xcodeDestinationsUsageStatistics": Record<string, number>; // destinationId -> usageCount
  "build.xcodeDestinationsRecent": SelectedDestination[];
  "build.xcodeSdk": string;
  "build.lastLaunchedApp": LastLaunchedAppContext;
  "build.xcodeBuildServerAutogenreateInfoShown": boolean;
  "testing.xcodeTarget": string;
  "testing.xcodeConfiguration": string;
  "testing.xcodeDestination": SelectedDestination;
  "testing.xcodeScheme": string;
};

type WorkspaceStateKey = keyof WorkspaceTypes;

/**
 * Global events that extension can emit
 */
type IEventMap = {
  executionScopeClosed: [scope: ExecutionScope];
  workspaceConfigChanged: [];
};
type IEventKey = keyof IEventMap;

/**
 * Global extension context that is used to manage the state of the extension.
 *
 * You can pass it everywhere in the extension to access full state of the extension.
 */
export class ExtensionContext {
  private _context: vscode.ExtensionContext;

  // Managers 💼
  // These classes are responsible for managing the state of the specific domain. Other parts of the extension can
  // interact with them to get the current state of the domain and subscribe to changes. For example
  // "DestinationsManager" have methods to get the list of current ios devices and simulators, and it also have an
  // event emitter that emits an event when the list of devices or simulators changes.
  public destinationsManager: DestinationsManager;
  public toolsManager: ToolsManager;
  public buildManager: BuildManager;
  public testingManager: TestingManager;
  public formatter: SwiftFormattingProvider;
  public progressStatusBar: ProgressStatusBar;
  public devicesManager: DevicesManager;
  public simulatorsManager: SimulatorsManager;

  public executionScope: ExecutionScopeManager;

  public emitter = new events.EventEmitter<IEventMap>();

  constructor(options: {
    context: vscode.ExtensionContext;
  }) {
    this._context = options.context;

    this.buildManager = new BuildManager({ context: this });
    this.devicesManager = new DevicesManager({ context: this });
    this.simulatorsManager = new SimulatorsManager();
    this.destinationsManager = new DestinationsManager({
      simulatorsManager: this.simulatorsManager,
      devicesManager: this.devicesManager,
      context: this,
    });
    this.toolsManager = new ToolsManager();
    this.testingManager = new TestingManager({ context: this });
    this.formatter = new SwiftFormattingProvider();
    this.progressStatusBar = new ProgressStatusBar({ context: this });

    this.executionScope = new ExecutionScopeManager({ context: this });

    vscode.workspace.onDidChangeConfiguration((event) => {
      const affected = event.affectsConfiguration("sweetpad");
      if (affected) {
        this.emitter.emit("workspaceConfigChanged");
      }
    });
  }

  get storageUri() {
    return this._context.storageUri;
  }

  get extensionPath() {
    return this._context.extensionPath;
  }

  disposable(disposable: vscode.Disposable) {
    this._context.subscriptions.push(disposable);
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
  }

  registerCommand(commandName: string, callback: (context: ExtensionContext, ...args: any[]) => Promise<unknown>) {
    return vscode.commands.registerCommand(commandName, async (...args: any[]) => {
      const commandContext = new CommandExecutionScope({ commandName: commandName });

      return await errorReporting.withScope(async (scope) => {
        return await this.executionScope.start(commandContext, async () => {
          scope.setTag("command", commandName);
          try {
            return await callback(this, ...args);
          } catch (error) {
            // User can cancel the quick pick dialog by pressing Escape or clicking outside of it.
            // In this case, we just stop the execution of the command and throw a QuickPickCancelledError.
            // Since it is more user action, then an error, we skip the error reporting.
            if (error instanceof QuickPickCancelledError) {
              return;
            }

            if (error instanceof ExtensionError) {
              // Handle default error
              commonLogger.error(error.message, {
                command: commandName,
                errorContext: error.options?.context,
                error: error,
              });
              if (error instanceof TaskError) {
                return; // do nothing
              }

              await this.showCommandErrorMessage(`SweetPad: ${error.message}`, {
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
            await this.showCommandErrorMessage(`SweetPad: ${errorMessage}`);
          }
        });
      });
    });
  }

  /**
   * Show error message with proper actions
   */
  async showCommandErrorMessage(
    message: string,
    options?: {
      actions?: ErrorMessageAction[];
    },
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

    const finalMessage = `${message}`;
    const action = await vscode.window.showErrorMessage(finalMessage, ...actionsLabels);

    if (action) {
      const callback = actions.find((a) => a.label === action)?.callback;
      if (callback) {
        callback();
      }
    }
  }

  registerTreeDataProvider<T extends vscode.TreeItem>(id: string, tree: vscode.TreeDataProvider<T>) {
    const wrappedTree = addTreeProviderErrorReporting(tree);
    return vscode.window.registerTreeDataProvider(id, wrappedTree);
  }

  /**
   * Migrate workspace state to the latest version
   */
  migrateWorkspaceState() {
    const currentVersion = this._context.workspaceState.get<number>("_version") ?? 0;
    let nextVersion = currentVersion + 1;

    // from version 0 to version 1
    if (nextVersion === 1) {
      // Nothing to migrate yet, but show how to update version
      this._context.workspaceState.update("_version", nextVersion);

      nextVersion += 1;
    }
  }

  updateWorkspaceState<T extends WorkspaceStateKey>(key: T, value: WorkspaceTypes[T] | undefined) {
    this._context.workspaceState.update(`sweetpad.${key}`, value);
  }

  getWorkspaceState<T extends WorkspaceStateKey>(key: T): WorkspaceTypes[T] | undefined {
    return this._context.workspaceState.get(`sweetpad.${key}`);
  }

  /**
   * Remove all sweetpad.* keys from workspace state
   */
  resetWorkspaceState() {
    for (const key of this._context.workspaceState.keys()) {
      if (key?.startsWith("sweetpad.")) {
        this._context.workspaceState.update(key, undefined);
      }
    }
    this.destinationsManager.setWorkspaceDestinationForBuild(undefined);
    this.destinationsManager.setWorkspaceDestinationForTesting(undefined);
    this.buildManager.setDefaultSchemeForBuild(undefined);
    this.buildManager.setDefaultSchemeForTesting(undefined);
    this.buildManager.setDefaultConfigurationForBuild(undefined);
    this.buildManager.setDefaultConfigurationForTesting(undefined);

    void this.buildManager.refreshSchemes();
    void this.destinationsManager.refresh();
  }

  updateProgressStatus(message: string) {
    this.progressStatusBar.updateText(message);
  }
}
