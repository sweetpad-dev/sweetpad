import * as vscode from "vscode";

import { getWorkspaceConfig, onDidChangeConfiguration } from "../common/config.js";
import type { PreviewsManager } from "./manager.js";

/**
 * Shows a "▶ Preview in VSCode" CodeLens above every `#Preview` / `PreviewProvider`
 * in an open Swift file. Clicking it streams that preview into the editor via
 * the preview host (see `sweetpad.previews.render`).
 */
export class PreviewsCodeLensProvider implements vscode.CodeLensProvider {
  private manager: PreviewsManager;

  private onDidChangeEmitter = new vscode.EventEmitter<void>();
  readonly onDidChangeCodeLenses = this.onDidChangeEmitter.event;

  constructor(options: { manager: PreviewsManager }) {
    this.manager = options.manager;
    // Re-emit lenses when the index changes (e.g. a preview was added/removed)
    // or when the CodeLens toggle is flipped.
    this.manager.onDidChange(() => this.onDidChangeEmitter.fire());
    onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("sweetpad.previews.codeLensEnabled")) {
        this.onDidChangeEmitter.fire();
      }
    });
  }

  provideCodeLenses(document: vscode.TextDocument): vscode.CodeLens[] {
    if (document.languageId !== "swift") return [];
    if (getWorkspaceConfig("previews.codeLensEnabled") === false) return [];

    const items = this.manager.itemsForDocument(document.uri, document.getText());
    return items.map((item) => {
      const position = new vscode.Position(item.match.line, item.match.character);
      const range = new vscode.Range(position, position);
      return new vscode.CodeLens(range, {
        title: "$(device-mobile) Preview in VSCode",
        command: "sweetpad.previews.render",
        arguments: [item],
      });
    });
  }
}
