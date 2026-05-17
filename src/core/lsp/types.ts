/**
 * Triggers a re-attach/restart of the Swift language server so it picks up
 * a freshly regenerated `buildServer.json`.
 *
 * - VS Code adapter: invokes `vscode.commands.executeCommand("swift.restartLSPServer")`.
 * - CLI adapter: no-op (sourcekit-lsp isn't running under the CLI).
 */
export interface LspRefresher {
  refresh(options?: { force?: boolean }): Promise<void>;
}

export const noopLspRefresher: LspRefresher = {
  async refresh() {},
};
