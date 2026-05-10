import * as vscode from "vscode";
import { getWorkspaceConfig } from "../config";

export function setTaskPresentationOptions(task: vscode.Task): void {
  const autoRevealTerminal = getWorkspaceConfig("system.autoRevealTerminal") ?? true;
  task.presentationOptions = {
    // terminal will be revealed, if auto reveal is enabled
    reveal: autoRevealTerminal ? vscode.TaskRevealKind.Always : vscode.TaskRevealKind.Never,
  };
}
