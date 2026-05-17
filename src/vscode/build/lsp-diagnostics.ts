import { type ChildProcess, spawn } from "node:child_process";
import { promises as fs } from "node:fs";

import * as vscode from "vscode";

import type { VsCodeWorkspaceState } from "../adapters/state";
import { commonLogger } from "../logger";

export const XBS_LOG_PATH = "/tmp/sweetpad-xbs.log";
const XBS_ENV_KEY = "XBS_LOGPATH";
const SOURCEKIT_ENV_KEY = "SOURCEKIT_LOGGING";
const SOURCEKIT_ENV_VALUE = "3";
const STATE_KEY = "build.lspDiagnosticsEnabled" as const;
const POST_RELOAD_KEY = "build.lspDiagnosticsPostReloadAction" as const;

/**
 * Diagnostics for "symbols not found" / sourcekit-lsp issues. Toggles verbose
 * logging on both xcode-build-server (XBS_LOGPATH) and sourcekit-lsp
 * (SOURCEKIT_LOGGING) and streams the XBS log file live into a dedicated
 * OutputChannel via `tail -F`. Enable/disable are explicit user actions —
 * nothing clears the env vars implicitly.
 */
export class LspDiagnosticsService implements vscode.Disposable {
  private outputChannel: vscode.OutputChannel | undefined;
  private tailProcess: ChildProcess | undefined;

  constructor(private readonly workspace: VsCodeWorkspaceState) {}

  /**
   * Enable verbose LSP diagnostics: sets env vars on both xcode-build-server
   * and sourcekit-lsp, marks the workspace state flag, truncates the log file,
   * and starts streaming it into the output channel.
   *
   * Caller is responsible for regenerating buildServer.json and restarting the
   * Swift LSP after this returns so the env vars take effect.
   */
  async enable(): Promise<void> {
    await this.setExtensionConfigEnv("sweetpad", "xcodebuildserver.serverEnv", XBS_ENV_KEY, XBS_LOG_PATH);
    await this.setExtensionConfigEnv("swift", "swiftEnvironmentVariables", SOURCEKIT_ENV_KEY, SOURCEKIT_ENV_VALUE);
    this.workspace.update(STATE_KEY, true);
    // Defer the user-facing notification to after the window reload triggered
    // by the calling command; the env-var changes don't take effect until
    // then, and racing with VS Code's own "reload required" prompt is ugly.
    this.workspace.update(POST_RELOAD_KEY, "enabled");
    await this.startTail();
  }

  /**
   * Undo the env-var writes and stop the tail. No-op if not currently enabled.
   *
   * Caller is responsible for regenerating buildServer.json and restarting the
   * Swift LSP — otherwise sourcekit-lsp keeps the previously-wrapped argv and
   * XBS continues logging until the next regen.
   */
  async disable(): Promise<void> {
    if (!this.workspace.get(STATE_KEY)) return;

    await this.setExtensionConfigEnv("sweetpad", "xcodebuildserver.serverEnv", XBS_ENV_KEY, undefined);
    await this.setExtensionConfigEnv("swift", "swiftEnvironmentVariables", SOURCEKIT_ENV_KEY, undefined);
    this.workspace.update(STATE_KEY, undefined);
    this.workspace.update(POST_RELOAD_KEY, "disabled");
    this.stopTail();
  }

  /**
   * Called on extension activation. If the user had diagnostics enabled in a
   * previous session, re-attach the tail so the stream resumes after a reload.
   */
  reattachIfEnabled(): void {
    if (!this.workspace.get(STATE_KEY)) return;
    void this.startTail();
  }

  /**
   * Called on activation right after `reattachIfEnabled`. If the previous
   * window reload was triggered by an enable/disable command, show the
   * corresponding success notification now (the command itself couldn't —
   * the window had to reload first to apply the env vars).
   */
  showPostReloadNotificationIfPending(): void {
    const action = this.workspace.get(POST_RELOAD_KEY);
    if (!action) return;
    this.workspace.update(POST_RELOAD_KEY, undefined);

    if (action === "enabled") {
      const openChannel = "Open Output Channel";
      vscode.window
        .showInformationMessage(
          `LSP diagnostics enabled. Check output panel: "SweetPad: XBS logs" and "SourceKit Language Server".`,
          openChannel,
        )
        .then((selected) => {
          if (selected === openChannel) this.showChannel();
        });
    } else {
      vscode.window.showInformationMessage("LSP diagnostics disabled.");
    }
  }

  dispose(): void {
    this.stopTail();
    this.outputChannel?.dispose();
    this.outputChannel = undefined;
  }

  /**
   * Reveal the SweetPad XBS logs output channel.
   */
  showChannel(): void {
    this.getChannel().show(true);
  }

  private getChannel(): vscode.OutputChannel {
    if (!this.outputChannel) {
      this.outputChannel = vscode.window.createOutputChannel("SweetPad: XBS logs");
    }
    return this.outputChannel;
  }

  private stopTail(): void {
    if (!this.tailProcess) return;
    try {
      this.tailProcess.kill("SIGTERM");
    } catch {
      // already gone
    }
    this.tailProcess = undefined;
  }

  private async startTail(): Promise<void> {
    this.stopTail();

    // Make sure the file exists so `tail -F` has something to attach to before
    // XBS writes its first line.
    try {
      await fs.writeFile(XBS_LOG_PATH, "", "utf8");
    } catch (e) {
      commonLogger.warn("Could not prepare XBS log file", {
        error: e instanceof Error ? e.message : String(e),
        path: XBS_LOG_PATH,
      });
      return;
    }

    const channel = this.getChannel();
    const proc = spawn("tail", ["-n", "0", "-F", XBS_LOG_PATH], {
      stdio: ["ignore", "pipe", "pipe"],
    });

    proc.stdout?.on("data", (data: Buffer) => {
      channel.append(data.toString());
    });
    proc.stderr?.on("data", (data: Buffer) => {
      channel.append(data.toString());
    });
    proc.on("error", (error) => {
      commonLogger.debug("Failed to tail XBS log file", { error: error.message, path: XBS_LOG_PATH });
    });

    this.tailProcess = proc;
  }

  private async setExtensionConfigEnv(
    section: string,
    configKey: string,
    envKey: string,
    envValue: string | undefined,
  ): Promise<void> {
    const config = vscode.workspace.getConfiguration(section);
    const existing = (config.get<Record<string, string | null>>(configKey) ?? {}) as Record<string, string | null>;
    const next: Record<string, string | null> = { ...existing };
    if (envValue === undefined) {
      delete next[envKey];
    } else {
      next[envKey] = envValue;
    }
    await config.update(configKey, next, vscode.ConfigurationTarget.Workspace);
  }
}
