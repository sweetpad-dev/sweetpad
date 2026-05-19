import * as net from "node:net";

import { readLineDelimitedJson } from "../server/rpc";
import type { JsonRpcRequest, JsonRpcResponse } from "../server/types";

const DEFAULT_TIMEOUT_MS = 6 * 60 * 1000;

// One-shot JSON-RPC 2.0 client over a Unix socket using newline-delimited framing.
export async function rpc<T = unknown>(options: {
  socketPath: string;
  method: string;
  params: unknown;
  timeoutMs?: number;
}): Promise<T> {
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const id = 1;
  const request: JsonRpcRequest = {
    jsonrpc: "2.0",
    id,
    method: options.method,
    params: options.params,
  };

  return await new Promise<T>((resolve, reject) => {
    const socket = net.createConnection(options.socketPath);
    let finished = false;
    const finish = (err: Error | undefined, value?: T) => {
      if (finished) return;
      finished = true;
      try {
        removeReader();
      } catch {
        // ignore
      }
      clearTimeout(timer);
      socket.destroy();
      if (err) reject(err);
      else resolve(value as T);
    };
    const timer = setTimeout(() => finish(new Error(`Request timed out after ${timeoutMs}ms`)), timeoutMs);
    timer.unref?.();

    const removeReader = readLineDelimitedJson(socket, (line) => {
      let parsed: JsonRpcResponse;
      try {
        parsed = JSON.parse(line) as JsonRpcResponse;
      } catch (err) {
        finish(new Error(`Invalid JSON from server: ${(err as Error).message}`));
        return;
      }
      if ((parsed as { id?: unknown }).id !== id) return;
      if ("error" in parsed && parsed.error) {
        const e = new RpcError(parsed.error.message, parsed.error.code, parsed.error.data);
        finish(e);
        return;
      }
      finish(undefined, (parsed as { result: T }).result);
    });

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
      socket.write(`${JSON.stringify(request)}\n`);
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
