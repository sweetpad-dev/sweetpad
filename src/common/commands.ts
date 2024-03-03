import * as vscode from "vscode";
import { ExtensionError, TaskError } from "./errors";
import { commonLogger } from "./logger";
import { isFileExists } from "./files";

type WorkspaceStateKey =
  | "build.xcodeWorkspacePath"
  | "build.xcodeProjectPath"
  | "build.xcodeScheme"
  | "build.xcodeConfiguration"
  | "build.xcodeSimulator"
  | "build.xcodeSdk";

/**
 * Class that represents a command execution with proper error handling
 */
export class CommandExecution {
  constructor(
    public readonly command: string,
    public readonly callback: (context: CommandExecution, ...args: any[]) => Promise<any>,
    public context: vscode.ExtensionContext
  ) {}

  /**
   * Show error message with proper actions
   */
  async showErrorMessage(
    message: string,
    options?: {
      withoutShowDetails: boolean;
    }
  ): Promise<void> {
    type Action = "Show details" | "Close";

    const actions: Action[] = options?.withoutShowDetails ? ["Close"] : ["Show details", "Close"];

    const finalMessage = `${message}`;
    const action = await vscode.window.showErrorMessage<Action>(finalMessage, ...actions);

    switch (action) {
      case "Show details":
        // Help user to find logs by showing the logs view
        commonLogger.show();
        break;
      case "Close" || undefined:
        break;
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
          errorContext: error.context,
        });
        if (error instanceof TaskError) {
          await this.showErrorMessage(`Sweetpad: ${error.message}. See "Terminal" output for details.`, {
            withoutShowDetails: true,
          });
        } else {
          await this.showErrorMessage(`Sweetpad: ${error.message}`);
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

  updateWorkspaceState(key: WorkspaceStateKey, value: any | undefined) {
    this.context.workspaceState.update(`sweetpad.${key}`, value);
  }

  getWorkspaceState<T = any>(key: WorkspaceStateKey): T | undefined {
    return this.context.workspaceState.get(`sweetpad.${key}`);
  }

  /**
   * Remove all sweetpad.* keys from workspace state
   */
  resetWorkspaceState() {
    this.context.workspaceState.keys().forEach((key) => {
      if (key.startsWith("sweetpad.")) {
        this.context.workspaceState.update(key, undefined);
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
}
