import * as vscode from "vscode";

import type { BuildManager } from "../build/manager";
import { getWorkspaceConfig, onDidChangeConfiguration } from "../common/config";
import { commonLogger } from "../common/logger";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DestinationsManager } from "../destination/manager";
import { BuildSessionRegistry } from "./builds";
import { buildDispatch } from "./handlers";
import { CliServer } from "./server";

const CONFIG_KEY_PREFIX = "sweetpad.";

/**
 * Pull every `sweetpad.*` configuration key out of the extension manifest so
 * `vscodeSettings.list` can enumerate them. The prefix is stripped — callers
 * pass keys in their post-prefix form (`build.xcbeautifyEnabled`) matching the
 * convention used by `getWorkspaceConfig`.
 */
function extractSweetpadConfigKeys(context: vscode.ExtensionContext): string[] {
  const properties: unknown = context.extension?.packageJSON?.contributes?.configuration?.properties;
  if (!properties || typeof properties !== "object") return [];
  return Object.keys(properties as Record<string, unknown>)
    .filter((k) => k.startsWith(CONFIG_KEY_PREFIX))
    .map((k) => k.slice(CONFIG_KEY_PREFIX.length))
    .toSorted();
}

/**
 * Owns the in-extension JSON-RPC server lifecycle — the control socket the CLI
 * connects to. Reads `sweetpad.cliServer.enabled` and starts/stops the server live
 * when that setting changes. Exposes the running server name so VS Code commands
 * can read it (e.g. for clipboard copy or status notifications). BSP is separate
 * (see `src/bsp`); this layer knows nothing about it.
 */
export class CliServerService implements vscode.Disposable {
  private readonly buildManager: BuildManager;
  private readonly destinationsManager: DestinationsManager;
  private readonly workspaceState: WorkspaceStateService;
  private readonly workspacePath: string;
  private readonly extensionVersion: string;
  private readonly vscodeContext: vscode.ExtensionContext;
  private readonly configKeys: string[];

  private current: { server: CliServer; registry: BuildSessionRegistry } | undefined;
  private configSubscription: vscode.Disposable | undefined;
  private starting = false;

  constructor(options: {
    buildManager: BuildManager;
    destinationsManager: DestinationsManager;
    workspaceState: WorkspaceStateService;
    workspacePath: string;
    extensionVersion: string;
    vscodeContext: vscode.ExtensionContext;
  }) {
    this.buildManager = options.buildManager;
    this.destinationsManager = options.destinationsManager;
    this.workspaceState = options.workspaceState;
    this.workspacePath = options.workspacePath;
    this.extensionVersion = options.extensionVersion;
    this.vscodeContext = options.vscodeContext;
    this.configKeys = extractSweetpadConfigKeys(options.vscodeContext);
  }

  async start(): Promise<void> {
    this.configSubscription = onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("sweetpad.cliServer.enabled")) {
        void this.reconcile();
      }
    });
    await this.reconcile();
  }

  async dispose(): Promise<void> {
    this.configSubscription?.dispose();
    this.configSubscription = undefined;
    await this.stop();
  }

  getStatus(): { running: boolean; name?: string; socket?: string } {
    if (!this.current) {
      return { running: false };
    }
    return {
      running: true,
      name: this.current.server.name,
      socket: this.current.server.socket,
    };
  }

  async restart(): Promise<void> {
    await this.stop();
    await this.reconcile();
  }

  private isEnabled(): boolean {
    return getWorkspaceConfig("cliServer.enabled") === true;
  }

  private async reconcile(): Promise<void> {
    if (this.starting) return;
    if (this.isEnabled()) {
      if (!this.current) {
        await this.startServer();
      }
    } else {
      await this.stop();
    }
  }

  private async startServer(): Promise<void> {
    this.starting = true;
    try {
      const workspacePath = this.workspacePath;
      const registry = new BuildSessionRegistry({
        workspacePath: workspacePath,
        buildManager: this.buildManager,
        destinationsManager: this.destinationsManager,
      });
      await registry.start();

      const dispatch = buildDispatch({
        workspacePath: workspacePath,
        extensionVersion: this.extensionVersion,
        workspaceState: this.workspaceState,
        buildManager: this.buildManager,
        destinationsManager: this.destinationsManager,
        buildRegistry: registry,
        vscodeContext: this.vscodeContext,
        configKeys: this.configKeys,
      });

      const server = new CliServer({
        workspacePath: workspacePath,
        extensionVersion: this.extensionVersion,
        handlers: dispatch,
      });
      try {
        await server.start();
      } catch (err) {
        registry.dispose();
        throw err;
      }
      this.current = { server, registry };
    } catch (err) {
      commonLogger.error("Failed to start SweetPad RPC server", { error: err });
    } finally {
      this.starting = false;
    }
  }

  private async stop(): Promise<void> {
    const current = this.current;
    this.current = undefined;
    if (!current) return;
    try {
      current.registry.dispose();
    } catch (err) {
      commonLogger.debug("BuildSessionRegistry.dispose threw", { error: err });
    }
    try {
      await current.server.dispose();
    } catch (err) {
      commonLogger.error("CliServer.dispose threw", { error: err });
    }
  }
}
