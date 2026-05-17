import * as net from "node:net";

import { MessageFramer, encodeMessage } from "../protocol/framing";
import type { MethodName, ParamsFor, ResultFor } from "../protocol/methods";
import { isResponse, type WireRequest, type WireResponse } from "../protocol/types";

export type ConnectOptions = {
  socketPath: string;
  /** Aborts the request if no response within this many ms. */
  requestTimeoutMs?: number;
};

/**
 * Newline-JSON client for the sweetpad-server socket. Single-request per
 * connection in v1 — open, send, await matching response, close. Events from
 * the server are ignored here (the first slice doesn't emit any).
 *
 * Method names + their param/result types are pulled from `protocol/methods.ts`,
 * so `client.request("build", { ... })` type-checks the params at the call
 * site and the return type carries the matching result shape.
 */
export class ProtocolClient {
  private constructor(
    private readonly socket: net.Socket,
    private readonly requestTimeoutMs: number,
  ) {}

  static async connect(socketPath: string): Promise<ProtocolClient> {
    return await new Promise<ProtocolClient>((resolve, reject) => {
      const socket = net.createConnection({ path: socketPath });
      socket.once("connect", () => {
        socket.removeAllListeners("error");
        resolve(new ProtocolClient(socket, 30 * 60 * 1000));
      });
      socket.once("error", (err) => reject(err));
    });
  }

  async request<M extends MethodName>(method: M, params: ParamsFor<M>): Promise<WireResponse<ResultFor<M>>> {
    const id = Math.floor(Math.random() * 0xffff_ffff);
    const request: WireRequest = { id, method, params: params as Record<string, unknown> };
    return await new Promise<WireResponse<ResultFor<M>>>((resolve, reject) => {
      const framer = new MessageFramer({
        onMessage: (message) => {
          if (!isResponse(message) || message.id !== id) return;
          cleanup();
          resolve(message as WireResponse<ResultFor<M>>);
        },
        onError: (line, error) => {
          cleanup();
          reject(new Error(`Malformed response: ${line} (${error instanceof Error ? error.message : String(error)})`));
        },
      });

      const onData = (chunk: Buffer) => framer.append(chunk.toString("utf8"));
      const onClose = () => {
        cleanup();
        reject(new Error("Server closed the connection before responding"));
      };
      const onError = (err: Error) => {
        cleanup();
        reject(err);
      };
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error(`Timed out waiting for '${method}' response`));
      }, this.requestTimeoutMs);

      const cleanup = () => {
        clearTimeout(timeout);
        this.socket.off("data", onData);
        this.socket.off("close", onClose);
        this.socket.off("error", onError);
      };

      this.socket.on("data", onData);
      this.socket.on("close", onClose);
      this.socket.on("error", onError);

      this.socket.write(encodeMessage(request));
    });
  }

  close(): void {
    this.socket.end();
  }
}
