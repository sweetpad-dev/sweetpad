import { CommandExecution } from "../common/commands";
import * as vscode from "vscode";

export async function resetSweetpadCache(execution: CommandExecution) {
  execution.resetWorkspaceState();
  vscode.window.showInformationMessage("Sweetpad cache has been reset");
}
