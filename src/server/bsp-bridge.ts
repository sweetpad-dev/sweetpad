import * as vscode from "vscode";
import type { MessageConnection } from "vscode-jsonrpc/node";

export type BspLogLevel = "off" | "error" | "info" | "debug";

export const BSP_LOG_LEVELS: readonly BspLogLevel[] = ["off", "error", "info", "debug"];

export type BspSnapshot = {
  connected: boolean;
  phase: string;
  detail?: string;
  level: BspLogLevel;
};

/**
 * Bridges the BSP server's control-channel notifications to VS Code UI: a
 * status-bar item reflecting the current phase, and an output channel streaming
 * its logs. Pushes the active log level down to every connected BSP server, so
 * verbosity is controllable live via the `bsp.setLogLevel` RPC / command.
 */
export class BspBridge implements vscode.Disposable {
  private readonly statusBar: vscode.StatusBarItem;
  private readonly output: vscode.OutputChannel;
  private readonly connections = new Set<MessageConnection>();
  private level: BspLogLevel = "info";
  private phase = "ready";
  private detail: string | undefined;

  constructor() {
    this.output = vscode.window.createOutputChannel("SweetPad BSP");
    this.statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    this.statusBar.command = "sweetpad.bsp.status";
    this.setPhase("ready", undefined);
  }

  /**
   * Wire a freshly-accepted connection. A connection becomes a tracked BSP
   * server the first time it emits a `bsp/*` notification (the CLI never does),
   * at which point it gets the current log level and the status bar appears.
   */
  attach(connection: MessageConnection): void {
    let identified = false;
    const identify = () => {
      if (identified) return;
      identified = true;
      this.connections.add(connection);
      this.statusBar.show();
      this.pushLevel(connection);
    };
    connection.onNotification("bsp/status", (params: { phase?: string; detail?: string | null }) => {
      identify();
      this.setPhase(params?.phase ?? "ready", params?.detail ?? undefined);
    });
    connection.onNotification("bsp/log", (params: { level?: string; message?: string }) => {
      identify();
      if (params?.message) {
        this.output.appendLine(`[${params.level ?? "info"}] ${params.message}`);
      }
    });
    connection.onClose(() => {
      this.connections.delete(connection);
      if (this.connections.size === 0) {
        this.statusBar.hide();
      }
    });
  }

  /** Set the log level and push it to every connected BSP server. */
  setLogLevel(level: BspLogLevel): void {
    this.level = level;
    for (const connection of this.connections) {
      this.pushLevel(connection);
    }
  }

  getLogLevel(): BspLogLevel {
    return this.level;
  }

  /** Reveal the "SweetPad BSP" output channel (without stealing focus). */
  revealLogs(): void {
    this.output.show(true);
  }

  snapshot(): BspSnapshot {
    return { connected: this.connections.size > 0, phase: this.phase, detail: this.detail, level: this.level };
  }

  private pushLevel(connection: MessageConnection): void {
    // Best-effort — a dropped connection must not throw into the RPC layer.
    void Promise.resolve(connection.sendNotification("bsp/setLogLevel", { level: this.level })).catch(() => {});
  }

  private setPhase(phase: string, detail: string | undefined): void {
    this.phase = phase;
    this.detail = detail;
    const icon = phase === "preparing" ? "$(sync~spin)" : phase === "error" ? "$(error)" : "$(check)";
    this.statusBar.text = `${icon} BSP`;
    this.statusBar.tooltip = `SweetPad BSP — ${detail ? `${phase}: ${detail}` : phase}`;
  }

  dispose(): void {
    this.statusBar.dispose();
    this.output.dispose();
    this.connections.clear();
  }
}
