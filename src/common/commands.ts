import * as vscode from "vscode";
import { ErrorMessageAction, ExtensionError, TaskError } from "./errors";
import { commonLogger } from "./logger";
import { isFileExists } from "./files";
import { BuildTreeProvider } from "../build/tree";
import { SimulatorsTreeProvider } from "../simulators/tree";
import { ToolTreeProvider } from "../tools/tree";

type WorkspaceStateKey =
  | "build.xcodeWorkspacePath"
  | "build.xcodeProjectPath"
  | "build.xcodeScheme"
  | "build.xcodeConfiguration"
  | "build.xcodeSimulator"
  | "build.xcodeSdk";

type SessionStateKey = "build.lastLaunchedAppPath";

export class ExtensionContext {
  private _context: vscode.ExtensionContext;
  public _buildProvider: BuildTreeProvider;
  public _simulatorsProvider: SimulatorsTreeProvider;
  public _toolsProvider: ToolTreeProvider;
  private _sessionState: Map<SessionStateKey, any> = new Map();

  constructor(options: {
    context: vscode.ExtensionContext;
    buildProvider: BuildTreeProvider;
    simulatorsProvider: SimulatorsTreeProvider;
    toolsProvider: ToolTreeProvider;
  }) {
    this._context = options.context;
    this._buildProvider = options.buildProvider;
    this._simulatorsProvider = options.simulatorsProvider;
    this._toolsProvider = options.toolsProvider;
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

  registerCommand(command: string, callback: (context: CommandExecution, ...args: any[]) => Promise<any>) {
    return vscode.commands.registerCommand(command, (...args: any[]) => {
      const execution = new CommandExecution(command, callback, this);
      return execution.run(...args);
    });
  }

  /**
   * State local to the running instance of the extension. It is not persisted across sessions.
   */
  updateSessionState(key: SessionStateKey, value: any | undefined) {
    this._sessionState.set(key, value);
  }

  getSessionState<T = any>(key: SessionStateKey): T | undefined {
    return this._sessionState.get(key);
  }

  updateWorkspaceState(key: WorkspaceStateKey, value: any | undefined) {
    this._context.workspaceState.update(`sweetpad.${key}`, value);
  }

  getWorkspaceState<T = any>(key: WorkspaceStateKey): T | undefined {
    return this._context.workspaceState.get(`sweetpad.${key}`);
  }

  /**
   * Remove all sweetpad.* keys from workspace state
   */
  resetWorkspaceState() {
    this._context.workspaceState.keys().forEach((key) => {
      if (key.startsWith("sweetpad.")) {
        this._context.workspaceState.update(key, undefined);
      }
    });
  }

  async withCache<T>(key: WorkspaceStateKey, callback: () => Promise<T>): Promise<T> {
    let value = this.getWorkspaceState<T>(key);
    if (value) {
      return value;
    }

    value = await callback();
    this.updateWorkspaceState(key, value);
    return value;
  }

  async withPathCache(key: WorkspaceStateKey, callback: () => Promise<string>): Promise<string> {
    let value = this.getWorkspaceState<string>(key);
    if (value) {
      if (!(await isFileExists(value))) {
        this.updateWorkspaceState(key, undefined);
      } else {
        return value;
      }
    }

    value = await callback();
    this.updateWorkspaceState(key, value);
    return value;
  }

  refreshSimulators() {
    this._simulatorsProvider.refresh();
  }
}

/**
 * Class that represents a command execution with proper error handling
 */
export class CommandExecution {
  constructor(
    public readonly command: string,
    public readonly callback: (context: CommandExecution, ...args: any[]) => Promise<any>,
    public context: ExtensionContext
  ) {}

  /**
   * Show error message with proper actions
   */
  async showErrorMessage(
    message: string,
    options?: {
      actions?: ErrorMessageAction[];
    }
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
  async run(...args: any[]) {
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
