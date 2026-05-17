import * as vscode from "vscode";

import type { AppDeps } from "../commands.js";
import { formatLogger } from "./logger.js";

/*
 * Format current opened document
 */
export async function formatCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Formatting document");
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    return;
  }

  const document = editor.document;
  await deps.formatter.formatDocument(document);
}

/*
 * Show "Output" panel with logs of the extension
 */
export async function showLogsCommand() {
  formatLogger.show();
}
