import * as vscode from "vscode";
import type { BuildManager } from "../build/manager";
import type { DestinationsManager } from "../destination/manager";
import type { SelectedDestination } from "../destination/types";
import type { ToolsManager } from "../tools/manager";
import { type ErrorMessageAction, ExtensionError, TaskError } from "./errors";
import { commonLogger } from "./logger";

type WorkspaceTypes = {
  "build.xcodeWorkspacePath": string;
  "build.xcodeProjectPath": string;
  "build.xcodeScheme": string;
  "build.xcodeConfiguration": string;
  "build.xcodeDestination": SelectedDestination;
  "build.xcodeDestinationsUsageStatistics": Record<string, number>;
  "build.xcodeSdk": string;
  "build.lastLaunchedAppPath": string;
};

type WorkspaceStateKey = keyof WorkspaceTypes;
type SessionStateKey = "NONE_KEY";

export class ExtensionContext {
  private _context: vscode.ExtensionContext;
  public destinationsManager: DestinationsManager;
  public toolsManager: ToolsManager;
  public buildManager: BuildManager;
  private _sessionState: Map<SessionStateKey, unknown> = new Map();

  constructor(options: {
    context: vscode.ExtensionContext;
    destinationsManager: DestinationsManager;
    buildManager: BuildManager;
    toolsManager: ToolsManager;
  }) {
    this._context = options.context;
    this.destinationsManager = options.destinationsManager;
    this.buildManager = options.buildManager;
    this.toolsManager = options.toolsManager;
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

  registerCommand(command: string, callback: (context: CommandExecution, ...args: any[]) => Promise<unknown>) {
    return vscode.commands.registerCommand(command, (...args: any[]) => {
      const execution = new CommandExecution(command, callback, this);
      return execution.run(...args);
    });
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
      if (key.startsWith("sweetpad.")) {
        this._context.workspaceState.update(key, undefined);
      }
    }
    this.destinationsManager.setWorkspaceDestination(undefined);
    this.buildManager.setDefaultScheme(undefined);

    this.buildManager.refresh();
    this.destinationsManager.refresh();
  }

  async withCache<T extends WorkspaceStateKey>(key: T, callback: () => Promise<WorkspaceTypes[T]>) {
    let value = this.getWorkspaceState<T>(key);
    if (value) {
      return value;
    }

    value = await callback();
    this.updateWorkspaceState(key, value);
    return value;
  }
}

/**
 * Class that represents a command execution with proper error handling
 */
export class CommandExecution {
  constructor(
    public readonly command: string,
    public readonly callback: (context: CommandExecution, ...args: unknown[]) => Promise<unknown>,
    public context: ExtensionContext,
  ) {}

  /**
   * Show error message with proper actions
   */
  async showErrorMessage(
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

  /**
   * Run the command with proper error handling. First argument passed to
   * the callback is this instance itself.
   */
  async run(...args: unknown[]) {
    try {
      return await this.callback(this, ...args);
    } catch (error) {
      if (error instanceof ExtensionError) {
        // Handle default error
        commonLogger.error(error.message, {
          command: this.command,
          errorContext: error.options?.context,
        });
        if (error instanceof TaskError) {
          // do nothing
        } else {
          await this.showErrorMessage(`Sweetpad: ${error.message}`, {
            actions: error.options?.actions,
          });
        }
      } else {
        // Handle unexpected error
        const errorMessage: string = error instanceof Error ? error.message : error?.toString() ?? "[unknown error]";
        commonLogger.error(errorMessage, {
          command: this.command,
          error: error,
        });
        await this.showErrorMessage(`Sweetpad: ${errorMessage}`);
      }
    }
  }
}
