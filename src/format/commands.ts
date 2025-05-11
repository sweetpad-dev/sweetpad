import * as vscode from "vscode";
import type { ExtensionContext } from "../common/commands.js";
import { formatLogger } from "./logger.js";

/*
 * Format current opened document
 */
export async function formatCommand(context: ExtensionContext) {
  context.updateProgressStatus("Formatting document");
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    return;
  }

  const document = editor.document;
  await context.formatter.formatDocument(document);
}

/*
 * Show "Output" panel with logs of the extension
 */
export async function showLogsCommand() {
  formatLogger.show();
}
