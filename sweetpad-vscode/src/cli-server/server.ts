import { promises as fs } from "node:fs";
import * as net from "node:net";

import type * as vscode from "vscode";
import type { MessageConnection } from "vscode-jsonrpc/node";

import { commonLogger } from "../common/logger";
import { generateServerName, getSocketPath, safeUnlink } from "./paths";
import { registerControlServer, unregisterControlServer } from "./registry";
import { serveDispatch, type RpcDispatch } from "./rpc";
import { PROTOCOL_VERSION, type CliServerMetadata } from "./types";

export type CliServerOptions = {
  /** The folder this server reports as its own in the discovery index metadata. */
  workspacePath: string;
  /**
   * Folders to advertise this server under. The CLI resolves a socket by walking up from its cwd
   * to the nearest registered ancestor, so in a multi-root workspace every folder needs an entry
   * pointing at this one socket — otherwise `sweetpad` run from an unlisted folder reports that no
   * server is running. Re-read on each sync so folders added to the window get an entry too.
   * Defaults to just `workspacePath`.
   */
  registrationPaths?: () => string[];
  extensionVersion: string;
  handlers: RpcDispatch;
  /** Called with each accepted connection before it starts listening (the BSP bridge hooks notifications here). */
  onConnection?: (connection: MessageConnection) => void;
};

/**
 * In-extension JSON-RPC 2.0 server. Listens on a per-process Unix socket in the
 * OS tmpdir (short path, for `sun_path`). Multi-window safe: each window starts
 * its own server with its own random name, the two coexist, and each advertises
 * itself in the host-wide discovery index (`registry`).
 *
 * Lifecycle owned by the caller — call `start()` once, `dispose()` once.
 */
export class CliServer implements vscode.Disposable {
  private readonly options: CliServerOptions;
  private readonly serverName: string;
  private readonly socketPath: string;
  private server: net.Server | undefined;
  private startedAt: Date | undefined;
  private connections = new Set<net.Socket>();
  private disposed = false;
  private metadata: CliServerMetadata | undefined;
  /** Folders currently advertising this server, so teardown retracts exactly what was published. */
  private readonly registeredPaths = new Set<string>();
  /** Serializes registration syncs — see `syncRegistrations`. */
  private syncQueue: Promise<void> = Promise.resolve();

  constructor(options: CliServerOptions) {
    this.options = options;
    this.serverName = generateServerName();
    this.socketPath = getSocketPath(this.serverName);
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

    this.metadata = {
      name: this.serverName,
      socket: this.socketPath,
      workspacePath: this.options.workspacePath,
      pid: process.pid,
      startedAt: this.startedAt.toISOString(),
      extensionVersion: this.options.extensionVersion,
      protocolVersion: PROTOCOL_VERSION,
    };
    await this.syncRegistrations();

    commonLogger.log("SweetPad RPC server started", {
      name: this.serverName,
      socket: this.socketPath,
      workspacePath: this.options.workspacePath,
    });
  }

  /**
   * Advertise this server in the host-wide discovery index under every folder it should be
   * reachable from, and retract the folders that no longer qualify. Last-writer-wins across
   * windows. Safe to call repeatedly — call it whenever the set of workspace folders changes.
   */
  async syncRegistrations(): Promise<void> {
    // Queued rather than run concurrently: two syncs in flight at once interleave their
    // index reads and writes, and one can drop a folder from `registeredPaths` that the
    // other has just re-published — leaving an entry nothing ever takes back down.
    const run = this.syncQueue.then(
      () => this.runSync(),
      () => this.runSync(),
    );
    // The queue itself must never stay rejected, or one failed sync wedges every later one.
    this.syncQueue = run.catch(() => {});
    return run;
  }

  private async runSync(): Promise<void> {
    const metadata = this.metadata;
    if (this.disposed || !metadata) return;

    const requested = this.options.registrationPaths?.() ?? [];
    const targets = new Set(requested.length > 0 ? requested : [this.options.workspacePath]);

    for (const stale of this.registeredPaths) {
      if (targets.has(stale)) continue;
      if (this.disposed) return;
      await unregisterControlServer(stale, process.pid);
      this.registeredPaths.delete(stale);
    }
    for (const target of targets) {
      // Re-checked before every write. `dispose()` waits for this run to finish, but a run
      // already in flight when it starts must not re-publish what it has just retracted.
      if (this.disposed) return;
      const claimed = await registerControlServer(target, metadata, {
        // Only the folder holding this window's project claims the key outright; the rest
        // step aside for a live window that owns the folder as its own project.
        secondary: target !== this.options.workspacePath,
      });
      if (claimed) {
        this.registeredPaths.add(target);
      } else {
        this.registeredPaths.delete(target);
      }
    }
  }

  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;

    // Settle any in-flight sync before retracting. It holds its own reference to the
    // metadata, so a run that started before this one would otherwise write the socket
    // back into the index after teardown had already removed it.
    await this.syncQueue;

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
    // Drop our control-server pointers (only where they still point at us — a newer
    // window may have replaced one under last-writer-wins).
    for (const workspacePath of this.registeredPaths) {
      await unregisterControlServer(workspacePath, process.pid);
    }
    this.registeredPaths.clear();
    this.metadata = undefined;

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
