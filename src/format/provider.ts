import * as vscode from "vscode";
import { formatDocument } from "./formatter.js";

/**
 * Register swiftpad as formatter for Swift documents. User then can use
 * "editor.defaultFormatter" in settings.json to set swiftpad as default formatter
 * for Swift documents.
 */
export function createFormatProvider() {
  return vscode.languages.registerDocumentFormattingEditProvider("swift", {
    provideDocumentFormattingEdits(document: vscode.TextDocument): vscode.ProviderResult<vscode.TextEdit[]> {
      formatDocument(document);
      return [];
    },
  });
}
