import * as vscode from "vscode";

const type = "sweetpad";
const source = "SweetPad";

export function createTaskProvider() {
  return vscode.tasks.registerTaskProvider(type, {
    provideTasks(token?: vscode.CancellationToken) {
      return [
        new vscode.Task(
          { type: type },
          vscode.TaskScope.Workspace,
          "Hello World",
          source,
          new vscode.ShellExecution('echo "Hello World"')
        ),
      ];
    },
    resolveTask(task: vscode.Task, token?: vscode.CancellationToken) {
      return task;
    },
  });
}
