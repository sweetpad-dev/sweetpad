import type { Socket } from "node:net";

import {
  createMessageConnection,
  type MessageConnection,
  ResponseError,
  StreamMessageReader,
  StreamMessageWriter,
} from "vscode-jsonrpc/node";

import { JSON_RPC_INTERNAL_ERROR, JSON_RPC_INVALID_PARAMS, SWEETPAD_APPLICATION_ERROR, type ErrorCode } from "./types";

/**
 * Error type that handlers throw to surface a structured RPC error with a
 * stable string code in `error.data.code`. The numeric JSON-RPC code stays in
 * the application-defined range (-32099..-32000), except INVALID_PARAMS which
 * maps to the reserved -32602.
 */
export class SweetpadRpcError extends Error {
  readonly code: ErrorCode;
  readonly hint?: string;
  readonly data?: Record<string, unknown>;

  constructor(code: ErrorCode, message: string, options?: { hint?: string; data?: Record<string, unknown> }) {
    super(message);
    this.code = code;
    this.hint = options?.hint;
    this.data = options?.data;
  }
}

export type RpcHandler = (params: unknown) => Promise<unknown> | unknown;
export type RpcDispatch = Record<string, RpcHandler>;

/** Wire shape of the `error.data` payload SweetPad puts on RPC failures. */
export type SweetpadErrorData = { code?: string; hint?: string; [key: string]: unknown };

/**
 * Map a thrown handler error to a vscode-jsonrpc `ResponseError`, preserving
 * the stable string code (+ optional hint and extra data) under `error.data`.
 * A `SweetpadRpcError` keeps its string code; anything else becomes an internal
 * error carrying the original message.
 */
export function toResponseError(error: unknown): ResponseError<SweetpadErrorData> {
  if (error instanceof SweetpadRpcError) {
    const numericCode = error.code === "INVALID_PARAMS" ? JSON_RPC_INVALID_PARAMS : SWEETPAD_APPLICATION_ERROR;
    return new ResponseError(numericCode, error.message, {
      code: error.code,
      ...(error.hint ? { hint: error.hint } : {}),
      ...error.data,
    });
  }
  const message = error instanceof Error ? error.message : String(error);
  return new ResponseError(JSON_RPC_INTERNAL_ERROR, message);
}

/**
 * Bind a connected socket to the dispatch table over Content-Length-framed
 * JSON-RPC 2.0. Each handler is registered as a request; unknown methods are
 * answered with -32601 by the connection itself. `onConnection` runs before
 * `listen()`, letting callers register extra handlers (e.g. the BSP bridge's
 * notification handlers). Returns the live connection — the caller owns disposal.
 */
export function serveDispatch(
  socket: Socket,
  handlers: RpcDispatch,
  onConnection?: (connection: MessageConnection) => void,
): MessageConnection {
  const connection = createMessageConnection(new StreamMessageReader(socket), new StreamMessageWriter(socket));
  for (const [method, handler] of Object.entries(handlers)) {
    connection.onRequest(method, async (params: unknown) => {
      try {
        return await handler(params ?? {});
      } catch (error) {
        throw toResponseError(error);
      }
    });
  }
  onConnection?.(connection);
  connection.listen();
  return connection;
}
