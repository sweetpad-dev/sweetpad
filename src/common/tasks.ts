import { exec } from "child_process";
import * as vscode from "vscode";

type Response =
  | {
      type: "success";
      stdout: string;
    }
  | {
      type: "error";
      error: string;
    };

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
}): Promise<Response> {
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
          reject({ type: "error", error: `Task failed with exit code ${e.exitCode}` });
        } else {
          resolve({ type: "success", stdout: "" });
        }
      }
    });
  });
}
