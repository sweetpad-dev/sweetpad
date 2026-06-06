import * as net from "node:net";

import {
  createMessageConnection,
  type MessageConnection,
  ResponseError,
  StreamMessageReader,
  StreamMessageWriter,
} from "vscode-jsonrpc/node";

const DEFAULT_TIMEOUT_MS = 6 * 60 * 1000;

// One-shot JSON-RPC 2.0 client over a Unix socket using Content-Length framing.
export async function rpc<T = unknown>(options: {
  socketPath: string;
  method: string;
  params: unknown;
  timeoutMs?: number;
}): Promise<T> {
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;

  return await new Promise<T>((resolve, reject) => {
    const socket = net.createConnection(options.socketPath);
    let connection: MessageConnection | undefined;
    let finished = false;
    const finish = (err: Error | undefined, value?: T) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      connection?.dispose();
      socket.destroy();
      if (err) reject(err);
      else resolve(value as T);
    };
    const timer = setTimeout(() => finish(new Error(`Request timed out after ${timeoutMs}ms`)), timeoutMs);
    timer.unref?.();

    socket.once("error", (err) => {
      const code = (err as NodeJS.ErrnoException)?.code;
      if (code === "ENOENT" || code === "ECONNREFUSED") {
        finish(
          new Error(`Cannot connect to server at ${options.socketPath} (${code}). Is the SweetPad RPC server running?`),
        );
        return;
      }
      finish(err);
    });

    socket.once("connect", () => {
      connection = createMessageConnection(new StreamMessageReader(socket), new StreamMessageWriter(socket));
      connection.onClose(() => finish(new Error("Connection closed before a response was received")));
      connection.listen();
      connection.sendRequest(options.method, options.params).then(
        (result) => finish(undefined, result as T),
        (err) => {
          if (err instanceof ResponseError) {
            finish(new RpcError(err.message, err.code, err.data as RpcError["data"]));
          } else {
            finish(err instanceof Error ? err : new Error(String(err)));
          }
        },
      );
    });
  });
}

export class RpcError extends Error {
  readonly code: number;
  readonly data?: { code?: string; hint?: string; [key: string]: unknown };
  constructor(message: string, code: number, data?: RpcError["data"]) {
    super(message);
    this.code = code;
    this.data = data;
  }
}
