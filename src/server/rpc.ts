import type { Socket } from "node:net";

import {
  JSON_RPC_INTERNAL_ERROR,
  JSON_RPC_INVALID_PARAMS,
  JSON_RPC_INVALID_REQUEST,
  JSON_RPC_METHOD_NOT_FOUND,
  JSON_RPC_PARSE_ERROR,
  SWEETPAD_APPLICATION_ERROR,
  type ErrorCode,
  type JsonRpcFailure,
  type JsonRpcId,
  type JsonRpcRequest,
  type JsonRpcResponse,
  type JsonRpcSuccess,
} from "./types";

/**
 * Error type that handlers throw to surface a structured RPC error with a
 * stable string code in `error.data.code`. The numeric JSON-RPC code stays in
 * the application-defined range (-32099..-32000).
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

/**
 * Parse one wire line as a JSON-RPC request. Returns either the request or a
 * failure envelope that should be sent back unmodified.
 */
export function parseRequest(line: string): JsonRpcRequest | JsonRpcFailure {
  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    return makeError(null, JSON_RPC_PARSE_ERROR, "Parse error");
  }

  if (
    !raw ||
    typeof raw !== "object" ||
    Array.isArray(raw) ||
    (raw as { jsonrpc?: unknown }).jsonrpc !== "2.0" ||
    typeof (raw as { method?: unknown }).method !== "string"
  ) {
    const id = extractId(raw);
    return makeError(id, JSON_RPC_INVALID_REQUEST, "Invalid Request");
  }

  return raw as JsonRpcRequest;
}

function extractId(raw: unknown): JsonRpcId {
  if (raw && typeof raw === "object" && "id" in raw) {
    const v = (raw as { id?: unknown }).id;
    if (typeof v === "string" || typeof v === "number" || v === null) {
      return v;
    }
  }
  return null;
}

/**
 * Run a parsed request through the dispatch table. Always resolves to a
 * response envelope — never throws.
 */
export async function dispatch(req: JsonRpcRequest, table: RpcDispatch): Promise<JsonRpcResponse> {
  const handler = table[req.method];
  if (!handler) {
    return makeError(req.id, JSON_RPC_METHOD_NOT_FOUND, `Method not found: ${req.method}`);
  }

  try {
    const result = await handler(req.params ?? {});
    return makeSuccess(req.id, result);
  } catch (error) {
    if (error instanceof SweetpadRpcError) {
      const numericCode = error.code === "INVALID_PARAMS" ? JSON_RPC_INVALID_PARAMS : SWEETPAD_APPLICATION_ERROR;
      return {
        jsonrpc: "2.0",
        id: req.id,
        error: {
          code: numericCode,
          message: error.message,
          data: {
            code: error.code,
            ...(error.hint ? { hint: error.hint } : {}),
            ...error.data,
          },
        },
      };
    }
    const message = error instanceof Error ? error.message : String(error);
    return makeError(req.id, JSON_RPC_INTERNAL_ERROR, message);
  }
}

function makeSuccess<T>(id: JsonRpcId, result: T): JsonRpcSuccess<T> {
  return { jsonrpc: "2.0", id, result };
}

function makeError(id: JsonRpcId, code: number, message: string): JsonRpcFailure {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

/**
 * Read newline-delimited JSON messages off a socket and invoke onMessage for
 * each. Buffers partial chunks across `data` events. Returns a disposer that
 * removes the listener.
 */
export function readLineDelimitedJson(socket: Socket, onMessage: (line: string) => void): () => void {
  let buffer = "";
  const handler = (chunk: Buffer) => {
    buffer += chunk.toString("utf8");
    let nlIdx: number;
    while ((nlIdx = buffer.indexOf("\n")) !== -1) {
      const line = buffer.slice(0, nlIdx).trim();
      buffer = buffer.slice(nlIdx + 1);
      if (line.length > 0) {
        onMessage(line);
      }
    }
  };
  socket.on("data", handler);
  return () => {
    socket.off("data", handler);
  };
}

export function writeMessage(socket: Socket, msg: JsonRpcResponse): void {
  socket.write(`${JSON.stringify(msg)}\n`);
}
