import * as vscode from "vscode";
import { formatDocument } from "./formatter.js";
import { formatLogger } from "./logger.js";

/*
 * Format current opened document
 */
export async function formatCommand() {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    return;
  }

  const document = editor.document;
  await formatDocument(document);
}

/*
 * Show "Output" panel with logs of the extension
 */
export async function showLogsCommand() {
  formatLogger.show();
}
