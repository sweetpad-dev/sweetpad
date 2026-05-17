import * as vscode from "vscode";

import { ExtensionError } from "../../core/errors";
import { createDirectory, getRelativePath } from "../../core/files";
import type { WorkspaceRoot } from "../../core/workspace-root";

/**
 * VS Code-backed WorkspaceRoot.
 *
 * - `getPath()` returns the first folder in `vscode.workspace.workspaceFolders`.
 * - `getStoragePath()` returns `vscode.ExtensionContext.storageUri.fsPath`,
 *   creating the directory on first use (VS Code allocates the URI but doesn't
 *   mkdir on disk).
 */
export class VsCodeWorkspaceRoot implements WorkspaceRoot {
  constructor(private readonly vscodeContext: vscode.ExtensionContext) {}

  getPath(): string {
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (!workspaceFolder) {
      throw new ExtensionError("No workspace folder found");
    }
    return workspaceFolder;
  }

  async getStoragePath(): Promise<string> {
    const storagePath = this.vscodeContext.storageUri?.fsPath;
    if (!storagePath) {
      throw new ExtensionError("No storage path found");
    }
    await createDirectory(storagePath);
    return storagePath;
  }

  getRelativePath(filePath: string): string {
    return getRelativePath(this.getPath(), filePath);
  }
}
