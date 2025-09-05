import { AsyncLocalStorage } from "node:async_hooks";
import * as crypto from "node:crypto";
import * as events from "node:events";
import * as vscode from "vscode";
import type { BuildManager } from "../build/manager";
import type { DestinationsManager } from "../destination/manager";
import type { DestinationType, SelectedDestination } from "../destination/types";
import type { SwiftFormattingProvider } from "../format/formatter";
import type { ProgressStatusBar } from "../system/status-bar";
import type { TestingManager } from "../testing/manager";
import type { ToolsManager } from "../tools/manager";
import { addTreeProviderErrorReporting, errorReporting } from "./error-reporting";
import { type ErrorMessageAction, ExtensionError, TaskError } from "./errors";
import { commonLogger } from "./logger";
import { QuickPickCancelledError } from "./quick-pick";
import * as path from "path";

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
  "bazel.selectedTarget": string;
};

type WorkspaceStateKey = keyof WorkspaceTypes;
type SessionStateKey = "NONE_KEY";

/**
 * Global events that extension can emit
 */
type IEventMap = {
  executionScopeClosed: [scope: ExecutionScope];
  workspaceConfigChanged: [];
};
type IEventKey = keyof IEventMap;

export class ExtensionContext {
  private _context: vscode.ExtensionContext;
  public destinationsManager: DestinationsManager;
  public toolsManager: ToolsManager;
  public buildManager: BuildManager;
  public testingManager: TestingManager;
  public formatter: SwiftFormattingProvider;
  public progressStatusBar: ProgressStatusBar;
  private _sessionState: Map<SessionStateKey, unknown> = new Map();

  // Create for each command and task execution separate execution scope with unique ID
  // to be able to track what is currently running
  private executionScope = new AsyncLocalStorage<ExecutionScope | undefined>();
  private emitter = new events.EventEmitter<IEventMap>();

  constructor(options: {
    context: vscode.ExtensionContext;
    destinationsManager: DestinationsManager;
    buildManager: BuildManager;
    toolsManager: ToolsManager;
    testingManager: TestingManager;
    formatter: SwiftFormattingProvider;
    progressStatusBar: ProgressStatusBar;
  }) {
    this._context = options.context;
    this.destinationsManager = options.destinationsManager;
    this.buildManager = options.buildManager;
    this.toolsManager = options.toolsManager;
    this.testingManager = options.testingManager;
    this.formatter = options.formatter;
    this.progressStatusBar = options.progressStatusBar;

    vscode.workspace.onDidChangeConfiguration((event) => {
      const affected = event.affectsConfiguration("sweetpad");
      if (affected) {
        this.emitter.emit("workspaceConfigChanged");
      }
    });
  }

  // --- Simple Global Emitter for Task Completion --- 
  public simpleTaskCompletionEmitter = new vscode.EventEmitter<void>();
  // ---------------------------------------------------

  // --- Define path for the UI log --- \
  public UI_LOG_PATH = (): string => {
    var workspaceFolders = vscode.workspace.workspaceFolders;
    if (workspaceFolders && workspaceFolders.length > 0) {
      // Construct the path relative to the first workspace folder
      return path.join(workspaceFolders[0].uri.fsPath, '.cursor', 'task_output.log');
    }
    // No workspace folder is open, cannot determine log path
    commonLogger.warn("Cannot determine UI log path: No workspace folder open.");
    return ""; // TODO: Handle this better
  };
  // ---------------------------------

  get storageUri() {
    return this._context.storageUri;
  }

  get extensionPath() {
    return this._context.extensionPath;
  }

  disposable(disposable: vscode.Disposable) {
    this._context.subscriptions.push(disposable);
  }

  /**
   * In case if you need to start propage execution scope manually you can use this method
   */
  setExecutionScope<T>(scope: ExecutionScope | undefined, callback: () => Promise<T>): Promise<T> {
    return this.executionScope.run(scope, callback);
  }

  getExecutionScope(): ExecutionScope | undefined {
    return this.executionScope.getStore();
  }

  getExecutionScopeId(): string | undefined {
    return this.getExecutionScope()?.id;
  }

  /**
   * Main method to start execution scope for command or task or other isolated execution context
   */
  startExecutionScope<T>(scope: ExecutionScope, callback: () => Promise<T>): Promise<T> {
    return this.executionScope.run(scope, async () => {
      try {
        return await callback();
      } finally {
        this.emitter.emit("executionScopeClosed", scope);
      }
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
  }

  registerCommand(commandName: string, callback: (context: ExtensionContext, ...args: any[]) => Promise<unknown>) {
    return vscode.commands.registerCommand(commandName, async (...args: any[]) => {
      const commandContext = new CommandExecutionScope({ commandName: commandName });

      return await errorReporting.withScope(async (scope) => {
        return await this.startExecutionScope(commandContext, async () => {
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
   * State local to the running instance of the extension. It is not persisted across sessions.
   */
  updateSessionState(key: SessionStateKey, value: unknown | undefined) {
    this._sessionState.set(key, value);
  }

  getSessionState<T = any>(key: SessionStateKey): T | undefined {
    return this._sessionState.get(key) as T | undefined;
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

    void this.buildManager.refresh();
    void this.destinationsManager.refresh();
  }

  updateProgressStatus(message: string) {
    this.progressStatusBar.updateText(message);
  }
}

export class CommandExecutionScope {
  id: string;
  type = "command" as const;
  commandName: string;

  constructor(options: { commandName: string }) {
    this.id = crypto.randomUUID();
    this.type = "command";
    this.commandName = options.commandName;
  }
}

export class TaskExecutionScope {
  id: string;
  type = "task" as const;
  taskName: string;

  constructor(options: { action: string }) {
    this.id = crypto.randomUUID();
    this.type = "task";
    this.taskName = options.action;
  }
}

export type ExecutionScope = CommandExecutionScope | TaskExecutionScope;
