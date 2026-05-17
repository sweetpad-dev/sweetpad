import type { ErrorCode } from "./error-codes";

export const SCHEMA_VERSION = "1.0";

export type WireRequest = {
  id: number;
  method: string;
  params?: Record<string, unknown>;
};

export type WireSuccessResponse<T = unknown> = {
  id: number;
  ok: true;
  schemaVersion: string;
  data: T;
};

export type WireErrorPayload = {
  code: ErrorCode;
  message: string;
  hint?: string;
};

export type WireErrorResponse = {
  id: number;
  ok: false;
  schemaVersion: string;
  error: WireErrorPayload;
  // Per-code extras (e.g. `running` list for BUILD_IN_PROGRESS).
  [extra: string]: unknown;
};

export type WireResponse<T = unknown> = WireSuccessResponse<T> | WireErrorResponse;

/**
 * Server-pushed event. No `id`, no response expected. Used by `attach` (live
 * stream) and replayed from disk for finished builds. Not used in the first
 * slice but the type lives here so server code can emit and CLI can ignore.
 */
export type WireEvent<T = unknown> = {
  event: string;
  schemaVersion: string;
  ts: string;
  buildId?: string;
  data: T;
};

export type WireMessage = WireRequest | WireResponse | WireEvent;

export function isResponse(msg: WireMessage): msg is WireResponse {
  return typeof (msg as WireResponse).id === "number" && "ok" in msg;
}

export function isEvent(msg: WireMessage): msg is WireEvent {
  return typeof (msg as WireEvent).event === "string" && !("id" in msg);
}

export function isRequest(msg: WireMessage): msg is WireRequest {
  return typeof (msg as WireRequest).method === "string" && typeof (msg as WireRequest).id === "number";
}

// ---------------------------------------------------------------------------
// Method: build
// ---------------------------------------------------------------------------

export type BuildRequestParams = {
  scheme: string;
  /** Destination ID or exact name. Server resolves against its destinations manager. */
  destination: string;
  configuration: string;
  /** Override the auto-detected xcworkspace path. Optional. */
  xcworkspace?: string;
  debug?: boolean;
};

export type Diagnostic = {
  file: string;
  line: number;
  column: number;
  endLine?: number;
  endColumn?: number;
  severity: "error" | "warning";
  message: string;
  source: string;
};

export type BuildStatus = "running" | "succeeded" | "failed" | "cancelled" | "interrupted";

export type BuildResponseData = {
  buildId: string;
  scheme: string;
  destination: string;
  config: string;
  command: "build";
  status: BuildStatus;
  exitCode: number | null;
  originator: "cli" | "vscode";
  startedAt: string;
  finishedAt: string | null;
  durationMs: number | null;
  errorCount: number;
  warningCount: number;
  diagnostics: Diagnostic[];
};
