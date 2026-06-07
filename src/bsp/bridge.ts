import * as net from "node:net";

import * as vscode from "vscode";
import {
  createMessageConnection,
  type MessageConnection,
  StreamMessageReader,
  StreamMessageWriter,
} from "vscode-jsonrpc/node";

export type BspLogLevel = "off" | "error" | "info" | "debug";

export const BSP_LOG_LEVELS: readonly BspLogLevel[] = ["off", "error", "info", "debug"];

export type BspSnapshot = {
  connected: boolean;
  detail?: string;
  level: BspLogLevel;
};

const RECONNECT_DELAY_MS = 1000;

/**
 * Bridges the BSP server's telemetry to VS Code UI: a status-bar item reflecting
 * the current state and an output channel streaming its logs. The BSP server
 * binds a Unix socket (the path the extension assigned in `bsp.json`); this dials
 * that socket, retrying until it's up and reconnecting if it drops, and pushes the
 * active log level down so verbosity is controllable live.
 */
export class BspBridge implements vscode.Disposable {
  private readonly output: vscode.OutputChannel;
  private level: BspLogLevel = "info";
  private detail: string | undefined;

  private socketPath: string | undefined;
  private socket: net.Socket | undefined;
  private connection: MessageConnection | undefined;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;

  constructor() {
    this.output = vscode.window.createOutputChannel("SweetPad: BSP");
  }

  /**
   * Connect to the BSP server's telemetry socket (the path the extension wrote
   * into `bsp.json`). Idempotent for the same path; retries until the server
   * binds it and reconnects if the connection later drops.
   */
  connect(socketPath: string): void {
    if (this.socketPath === socketPath && (this.socket || this.connection)) {
      return;
    }
    this.socketPath = socketPath;
    this.clearReconnect();
    this.teardownConnection();
    this.openConnection();
  }

  /** Stop connecting and tear down any live connection (no auto-reconnect). */
  disconnect(): void {
    this.socketPath = undefined;
    this.clearReconnect();
    this.teardownConnection();
  }

  private openConnection(): void {
    if (this.disposed || !this.socketPath || this.socket) {
      return;
    }
    const socket = net.connect(this.socketPath);
    this.socket = socket;
    socket.on("connect", () => {
      const reader = new StreamMessageReader(socket);
      const writer = new StreamMessageWriter(socket);
      const connection = createMessageConnection(reader, writer);
      this.connection = connection;
      connection.onNotification("bsp/log", (params: { level?: string; message?: string }) => {
        if (params?.message) {
          this.output.appendLine(`[${params.level ?? "info"}] ${params.message}`);
        }
      });
      connection.onClose(() => this.handleDrop());
      connection.onError(() => this.handleDrop());
      connection.listen();
      this.pushLevel();
    });
    // A missing socket (server not up yet) or a dropped one both retry.
    socket.on("error", () => this.handleDrop());
    socket.on("close", () => this.handleDrop());
  }

  private handleDrop(): void {
    this.teardownConnection();
    this.scheduleReconnect();
  }

  private teardownConnection(): void {
    this.connection?.dispose();
    this.connection = undefined;
    if (this.socket) {
      this.socket.removeAllListeners();
      this.socket.destroy();
      this.socket = undefined;
    }
  }

  private scheduleReconnect(): void {
    if (this.disposed || !this.socketPath || this.reconnectTimer) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.openConnection();
    }, RECONNECT_DELAY_MS);
  }

  private clearReconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
    }
  }

  setLogLevel(level: BspLogLevel): void {
    this.level = level;
    this.pushLevel();
  }

  revealLogs(): void {
    this.output.show(true);
  }

  /** Append a report block (e.g. the Doctor checklist) and reveal the channel. */
  report(lines: string[]): void {
    this.output.appendLine("");
    for (const line of lines) {
      this.output.appendLine(line);
    }
    this.output.show(true);
  }

  snapshot(): BspSnapshot {
    return {
      connected: this.connection !== undefined,
      detail: this.detail,
      level: this.level,
    };
  }

  private pushLevel(): void {
    // Best-effort — a dropped connection must not throw into the caller.
    if (!this.connection) return;
    void Promise.resolve(this.connection.sendNotification("bsp/setLogLevel", { level: this.level })).catch(() => {});
  }

  dispose(): void {
    this.disposed = true;
    this.disconnect();
    this.output.dispose();
  }
}
