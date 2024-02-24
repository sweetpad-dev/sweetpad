import * as vscode from "vscode";
import { TaskError } from "./errors";

/**
 * Runs a shell task asynchronously.
 * @param options - The options for the shell task.
 * @returns A promise that resolves when the task is completed successfully, or rejects with the exit code if the task fails.
 */
export async function runShellTask(options: {
  name: string;
  source?: string;
  command: string;
  args: string[];
  error?: string;
}): Promise<void> {
  const task = new vscode.Task(
    { type: "shell" },
    vscode.TaskScope.Workspace,
    options.name,
    options.source ?? "sweetpad",
    new vscode.ShellExecution(options.command, options.args)
  );

  const execution = await vscode.tasks.executeTask(task);

  return new Promise((resolve, reject) => {
    const disposable = vscode.tasks.onDidEndTaskProcess((e) => {
      if (e.execution === execution) {
        disposable.dispose();
        if (e.exitCode !== 0) {
          const message = options.error ?? `Error running task '${options.name}'`;
          const error = new TaskError(message, {
            name: options.name,
            soruce: options.source,
            command: options.command,
            args: options.args,
            errorCode: e.exitCode,
          });
          reject(error);
        } else {
          resolve();
        }
      }
    });
  });
}
