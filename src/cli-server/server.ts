import { promises as fs } from "node:fs";
import * as net from "node:net";

import type * as vscode from "vscode";
import type { MessageConnection } from "vscode-jsonrpc/node";

import { commonLogger } from "../common/logger";
import { ensureDir, generateServerName, getCliConfigFile, getSocketPath, getStateRoot, safeUnlink } from "./paths";
import { serveDispatch, type RpcDispatch } from "./rpc";
import { PROTOCOL_VERSION, type CliServerMetadata } from "./types";

export type CliServerOptions = {
  workspacePath: string;
  extensionVersion: string;
  handlers: RpcDispatch;
  /** Called with each accepted connection before it starts listening (the BSP bridge hooks notifications here). */
  onConnection?: (connection: MessageConnection) => void;
};

/**
 * In-extension JSON-RPC 2.0 server. Listens on a per-process Unix socket under
 * ~/.local/state/sweetpad/sockets/. Multi-window safe: each window starts its
 * own server with its own random name, the two coexist.
 *
 * Lifecycle owned by the caller — call `start()` once, `dispose()` once.
 */
export class CliServer implements vscode.Disposable {
  private readonly options: CliServerOptions;
  private readonly serverName: string;
  private readonly socketPath: string;
  private readonly connectionPath: string;
  private server: net.Server | undefined;
  private startedAt: Date | undefined;
  private connections = new Set<net.Socket>();
  private disposed = false;

  constructor(options: CliServerOptions) {
    this.options = options;
    this.serverName = generateServerName();
    this.socketPath = getSocketPath(this.serverName);
    this.connectionPath = getCliConfigFile(options.workspacePath);
  }

  get name(): string {
    return this.serverName;
  }

  get socket(): string {
    return this.socketPath;
  }

  async start(): Promise<void> {
    if (this.server) {
      throw new Error("CliServer.start() called twice");
    }
    await ensureDir(getStateRoot(this.options.workspacePath));
    // Defensive — without this a rare same-name collision would hit EADDRINUSE.
    await safeUnlink(this.socketPath);

    const server = net.createServer((socket) => this.onConnection(socket));
    server.on("error", (err) => {
      commonLogger.error("SweetPad RPC server error", { error: err });
    });

    await new Promise<void>((resolve, reject) => {
      const onError = (err: Error) => reject(err);
      server.once("error", onError);
      server.listen({ path: this.socketPath, exclusive: true }, () => {
        server.off("error", onError);
        resolve();
      });
    });

    // Restrict to the owning user via filesystem perms.
    try {
      await fs.chmod(this.socketPath, 0o600);
    } catch (err) {
      commonLogger.warn("Failed to chmod 0600 the sweetpad socket", { error: err });
    }

    this.server = server;
    this.startedAt = new Date();

    const metadata: CliServerMetadata = {
      name: this.serverName,
      socket: this.socketPath,
      workspacePath: this.options.workspacePath,
      pid: process.pid,
      startedAt: this.startedAt.toISOString(),
      extensionVersion: this.options.extensionVersion,
      protocolVersion: PROTOCOL_VERSION,
    };
    // Last-writer-wins: a second window overwrites cli.json. tmp+rename keeps the
    // CLI's read side from ever seeing a half-written file.
    const tmp = `${this.connectionPath}.${process.pid}.tmp`;
    await fs.writeFile(tmp, JSON.stringify(metadata, null, 2), { mode: 0o600 });
    await fs.rename(tmp, this.connectionPath);

    commonLogger.log("SweetPad RPC server started", {
      name: this.serverName,
      socket: this.socketPath,
      workspacePath: this.options.workspacePath,
    });
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;

    for (const conn of this.connections) {
      conn.destroy();
    }
    this.connections.clear();

    const server = this.server;
    this.server = undefined;
    if (server) {
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }

    await safeUnlink(this.socketPath);
    // Remove cli.json only if it still points at us — a newer window may have
    // overwritten it (last-wins), and we must not delete its pointer.
    try {
      const raw = await fs.readFile(this.connectionPath, "utf8");
      if ((JSON.parse(raw) as CliServerMetadata).pid === process.pid) {
        await safeUnlink(this.connectionPath);
      }
    } catch {
      // missing or corrupt — nothing of ours to remove
    }

    commonLogger.log("SweetPad RPC server stopped", { name: this.serverName });
  }

  private onConnection(socket: net.Socket): void {
    this.connections.add(socket);

    const connection = serveDispatch(socket, this.options.handlers, this.options.onConnection);

    const cleanup = () => {
      connection.dispose();
      this.connections.delete(socket);
    };
    socket.once("close", cleanup);
    socket.once("error", (err) => {
      commonLogger.debug("SweetPad RPC client connection error", { error: err });
      cleanup();
    });
  }
}
