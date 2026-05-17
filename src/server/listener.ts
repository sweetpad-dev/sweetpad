import * as fs from "node:fs";
import * as net from "node:net";
import * as path from "node:path";

import type { Logger } from "../core/logger/types";
import { MessageFramer, encodeMessage } from "../protocol/framing";
import { isRequest, type WireMessage } from "../protocol/types";
import type { MethodDispatcher } from "./dispatcher";

const SOCKET_DIR_MODE = 0o700;
const SOCKET_FILE_MODE = 0o600;

export type ListenerOptions = {
  socketPath: string;
  dispatcher: MethodDispatcher;
  logger: Logger;
  /** Notified when connection count transitions to or from zero. */
  onActiveChange?: (activeConnections: number) => void;
};

/**
 * Owns the Unix domain socket the CLI clients connect to. Each connection
 * gets its own newline-JSON framer; the listener parses requests, hands them
 * to the dispatcher, and writes the response back. Malformed lines are logged
 * and dropped (the connection stays open — a bad request shouldn't kill it).
 */
export class Listener {
  private readonly server: net.Server;
  private readonly sockets = new Set<net.Socket>();
  private readonly onActiveChange: ((n: number) => void) | undefined;
  private listening = false;

  constructor(private readonly options: ListenerOptions) {
    this.onActiveChange = options.onActiveChange;
    this.server = net.createServer((socket) => this.handleConnection(socket));
    this.server.on("error", (err) => {
      options.logger.error("Listener server error", { error: err });
    });
  }

  async listen(): Promise<void> {
    const socketPath = this.options.socketPath;
    // Caller (`tryAcquireLock`) should have removed any stale socket already.
    // Ensure the run-dir exists with restrictive perms — multi-user macOS
    // shouldn't expose one user's sockets to another.
    fs.mkdirSync(path.dirname(socketPath), { recursive: true, mode: SOCKET_DIR_MODE });

    await new Promise<void>((resolve, reject) => {
      const onError = (err: NodeJS.ErrnoException) => {
        this.server.removeListener("listening", onListening);
        reject(err);
      };
      const onListening = () => {
        this.server.removeListener("error", onError);
        try {
          fs.chmodSync(socketPath, SOCKET_FILE_MODE);
        } catch {
          // Non-fatal — restricted parent dir already protects us.
        }
        this.listening = true;
        resolve();
      };
      this.server.once("error", onError);
      this.server.once("listening", onListening);
      this.server.listen(socketPath);
    });
  }

  /** Currently-open client connections. */
  activeConnections(): number {
    return this.sockets.size;
  }

  async close(): Promise<void> {
    if (!this.listening) return;
    for (const socket of this.sockets) socket.end();
    await new Promise<void>((resolve) => {
      this.server.close(() => resolve());
    });
    this.listening = false;
  }

  private handleConnection(socket: net.Socket): void {
    this.sockets.add(socket);
    this.onActiveChange?.(this.sockets.size);

    const framer = new MessageFramer({
      onMessage: (message) => void this.handleMessage(socket, message),
      onError: (line, error) => {
        this.options.logger.warn("Discarding malformed message", {
          line,
          error: error instanceof Error ? error.message : String(error),
        });
      },
    });

    socket.on("data", (chunk: Buffer) => framer.append(chunk));
    socket.once("close", () => {
      framer.flush();
      this.sockets.delete(socket);
      this.onActiveChange?.(this.sockets.size);
    });
    socket.on("error", (err) => {
      this.options.logger.debug("Socket error", { error: err.message });
    });
  }

  private async handleMessage(socket: net.Socket, message: WireMessage): Promise<void> {
    if (!isRequest(message)) {
      // Server only consumes requests; ignore responses/events the client may have sent in error.
      return;
    }
    const response = await this.options.dispatcher.handle(message);
    if (!socket.writable) return;
    socket.write(encodeMessage(response));
  }
}
