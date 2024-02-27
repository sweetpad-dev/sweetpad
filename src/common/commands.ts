import * as vscode from "vscode";
import { ExtensionError, TaskError } from "./errors";
import { commonLogger } from "./logger";

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
          await this.showErrorMessage(`Sweetpad: ${error.message}. See "Terminal" output for details.`);
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

  get xcodeWorkspacePath(): string | undefined {
    return this.context.workspaceState.get<string>("sweetpad.build.xcodeWorkspacePath");
  }
  set xcodeWorkspacePath(value: string | undefined) {
    this.context.workspaceState.update("sweetpad.build.xcodeWorkspacePath", value);
  }
}
