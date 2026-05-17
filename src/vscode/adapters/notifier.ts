import * as vscode from "vscode";

import type { Notifier } from "../../core/notifier/types";

/**
 * VS Code-backed Notifier. Engine-raised messages surface as `showInformationMessage`,
 * `showWarningMessage`, and `showErrorMessage` toasts (each prefixed with "SweetPad:").
 */
export class VsCodeNotifier implements Notifier {
  info(message: string): void {
    void vscode.window.showInformationMessage(`SweetPad: ${message}`);
  }

  warn(message: string): void {
    void vscode.window.showWarningMessage(`SweetPad: ${message}`);
  }

  error(message: string): void {
    void vscode.window.showErrorMessage(`SweetPad: ${message}`);
  }
}
