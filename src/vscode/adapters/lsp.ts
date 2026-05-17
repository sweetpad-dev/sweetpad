import * as vscode from "vscode";

import type { ConfigProvider } from "../../core/config/types";
import type { Logger } from "../../core/logger/types";
import type { LspRefresher } from "../../core/lsp/types";

/**
 * VS Code-backed LspRefresher.
 *
 * Honors `sweetpad.build.autoRestartSwiftLSP` (default true). Pass `{ force: true }` from
 * explicitly user-invoked commands (e.g. "Generate Build Server Config") that should
 * always restart regardless of the setting.
 */
export class VsCodeLspRefresher implements LspRefresher {
  constructor(
    private readonly config: ConfigProvider,
    private readonly logger: Logger,
  ) {}

  async refresh(options?: { force?: boolean }): Promise<void> {
    if (!options?.force) {
      const isEnabled = this.config.get("build.autoRestartSwiftLSP") ?? true;
      if (!isEnabled) {
        this.logger.debug("Skipping Swift LSP restart (build.autoRestartSwiftLSP is false)");
        return;
      }
    }

    try {
      await vscode.commands.executeCommand("swift.restartLSPServer");
    } catch (error) {
      this.logger.warn("Error restarting SourceKit Language Server", { error });
    }
  }
}
