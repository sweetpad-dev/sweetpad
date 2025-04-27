import * as vscode from "vscode";
import type { CommandExecution } from "../common/commands.js";
import { formatLogger } from "./logger.js";

/*
 * Format current opened document
 */
export async function formatCommand(execution: CommandExecution) {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    return;
  }

  const document = editor.document;
  await execution.context.formatter.formatDocument(document);
}

/*
 * Show "Output" panel with logs of the extension
 */
export async function showLogsCommand() {
  formatLogger.show();
}
