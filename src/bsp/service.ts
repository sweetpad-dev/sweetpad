import { promises as fs } from "node:fs";

import * as vscode from "vscode";

import type { BuildManager } from "../build/manager";
import { getWorkspacePath } from "../build/utils";
import { ensureDir, getProjectStateDir } from "../cli-server/paths";
import { registerBspConfig, unregisterBspConfig } from "../cli-server/registry";
import { getWorkspaceConfig, onDidChangeConfiguration } from "../common/config";
import { commonLogger } from "../common/logger";
import type { WorkspaceStateService } from "../common/workspace-state";
import { BSP_LOG_LEVELS, BspBridge, type BspLogLevel } from "./bridge";
import { getBuildServerProvider, isSweetpadBuildServerActive } from "./commands";
import { buildBspResolvedConfig } from "./config";
import { getBspConfigFile, getBspSocketPath } from "./paths";

export type BspStatusSnapshot = {
  bspConnected: boolean;
  scheme: string | null;
  configuration: string | null;
  logLevel: BspLogLevel;
};

/**
 * Owns the BSP side end to end, independent of the CLI control server: persists
 * the per-project `bsp.json` (under the XDG state home) for the BSP server to
 * read, and dials the server's telemetry socket to surface its logs/status in
 * VS Code. Activates whenever
 * SweetPad is the build-server provider for the open workspace — it does not
 * depend on `sweetpad.cliServer.enabled`.
 */
export class BspService implements vscode.Disposable {
  private readonly bridge = new BspBridge();
  private readonly buildManager: BuildManager;
  private readonly workspaceState: WorkspaceStateService;
  private subscriptions: vscode.Disposable[] = [];

  constructor(options: { buildManager: BuildManager; workspaceState: WorkspaceStateService }) {
    this.buildManager = options.buildManager;
    this.workspaceState = options.workspaceState;
  }

  async start(): Promise<void> {
    this.buildManager.on("defaultSchemeForBuildUpdated", () => this.saveConfig());
    this.buildManager.on("defaultConfigurationForBuildUpdated", () => this.saveConfig());

    this.subscriptions.push(
      onDidChangeConfiguration((event) => {
        if (event.affectsConfiguration("sweetpad.buildServer.provider")) void this.activate();
        if (event.affectsConfiguration("sweetpad.buildServer.logLevel")) this.applyLogLevel();
      }),
    );
    this.applyLogLevel();
    void this.activate();
  }

  private async activate(): Promise<void> {
    if (getBuildServerProvider() !== "sweetpad") {
      this.bridge.disconnect();
      return;
    }
    const workspacePath = getWorkspacePath();
    const bspSocket = getBspSocketPath(workspacePath);

    await this.saveConfig();
    this.bridge.connect(bspSocket);
  }

  /**
   * Persist the resolved config to the per-project `bsp.json` (under the XDG
   * state home), only when SweetPad is the provider and buildServer.json exists
   * (otherwise sourcekit-lsp won't launch our server, so the file is moot).
   * Best-effort — a write failure or a folder with no Xcode workspace is logged,
   * not surfaced.
   */
  private async saveConfig(): Promise<void> {
    const workspacePath = getWorkspacePath();
    try {
      const isActive = await isSweetpadBuildServerActive(workspacePath);
      if (!isActive) {
        return;
      }

      const config = await buildBspResolvedConfig({
        workspaceState: this.workspaceState,
        workspacePath: workspacePath,
        buildManager: this.buildManager,
      });
      if (!config) {
        return;
      }
      await ensureDir(getProjectStateDir(workspacePath));
      const configFile = getBspConfigFile(workspacePath);
      await fs.writeFile(configFile, `${JSON.stringify(config, null, 2)}\n`, { mode: 0o600 });
      // Advertise the config path so the BSP server can find it from its cwd when
      // buildServer.json carries no explicit `--config` (an older or hand-written
      // stub). Independent of the control server's index entry.
      await registerBspConfig(workspacePath, configFile);
    } catch (err) {
      commonLogger.debug("Failed to write bsp.json", { error: err });
    }
  }

  /**
   * Push the configured `sweetpad.buildServer.logLevel` to the bridge, which
   * forwards it to the connected BSP server and re-sends it on reconnect.
   */
  private applyLogLevel(): void {
    const level = getWorkspaceConfig("buildServer.logLevel");
    this.bridge.setLogLevel(level && BSP_LOG_LEVELS.includes(level) ? level : "info");
  }

  revealLogs(): void {
    this.bridge.revealLogs();
  }

  writeReport(lines: string[]): void {
    this.bridge.report(lines);
  }

  snapshot(): BspStatusSnapshot {
    const b = this.bridge.snapshot();
    return {
      bspConnected: b.connected,
      scheme: this.buildManager.getDefaultSchemeForBuild() ?? null,
      configuration: this.buildManager.getDefaultConfigurationForBuild() ?? null,
      logLevel: b.level,
    };
  }

  dispose(): void {
    this.buildManager.removeAllListeners("defaultSchemeForBuildUpdated");
    this.buildManager.removeAllListeners("defaultConfigurationForBuildUpdated");

    // Best-effort: drop our BSP pointer from the discovery index. Fire-and-forget
    // — a missing workspace or write failure must not block teardown.
    try {
      void unregisterBspConfig(getWorkspacePath()).catch(() => {});
    } catch {
      // no workspace to unregister
    }

    for (const s of this.subscriptions) s.dispose();
    this.subscriptions = [];
    this.bridge.dispose();
  }
}
