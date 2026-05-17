/**
 * Abstraction over "where the project lives" and "where the engine can stash files".
 *
 * - VS Code adapter: getPath() returns vscode.workspace.workspaceFolders[0].uri.fsPath,
 *   getStoragePath() returns vscodeContext.storageUri.fsPath.
 * - CLI adapter: getPath() returns process.cwd() (walked to the workspace root),
 *   getStoragePath() returns <project>/.sweetpad/run/<workspace-hash>/.
 */
export interface WorkspaceRoot {
  getPath(): string;
  getStoragePath(): Promise<string>;
  getRelativePath(filePath: string): string;
}
