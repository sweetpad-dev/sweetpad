/**
 * Shared wire-protocol types. Imported by both the extension server and the
 * CLI client — must not pull in `vscode` or any extension-only module.
 */

export const PROTOCOL_VERSION = "1.0";

export type JsonRpcId = string | number | null;

export type JsonRpcRequest = {
  jsonrpc: "2.0";
  id: JsonRpcId;
  method: string;
  params?: unknown;
};

export type JsonRpcSuccess<T = unknown> = {
  jsonrpc: "2.0";
  id: JsonRpcId;
  result: T;
};

export type JsonRpcErrorPayload = {
  code: number;
  message: string;
  data?: {
    code?: string;
    hint?: string;
    [key: string]: unknown;
  };
};

export type JsonRpcFailure = {
  jsonrpc: "2.0";
  id: JsonRpcId;
  error: JsonRpcErrorPayload;
};

export type JsonRpcResponse<T = unknown> = JsonRpcSuccess<T> | JsonRpcFailure;

// JSON-RPC reserved error codes
export const JSON_RPC_PARSE_ERROR = -32700;
export const JSON_RPC_INVALID_REQUEST = -32600;
export const JSON_RPC_METHOD_NOT_FOUND = -32601;
export const JSON_RPC_INVALID_PARAMS = -32602;
export const JSON_RPC_INTERNAL_ERROR = -32603;
// Application-defined error codes live in -32099..-32000
export const SWEETPAD_APPLICATION_ERROR = -32000;

export type ServerMetadata = {
  name: string;
  workspacePath: string;
  pid: number;
  startedAt: string;
  extensionVersion: string;
  protocolVersion: string;
};

export type ActiveServer = {
  server: string;
  setAt: string;
};

export type SchemeEntity = {
  name: string;
  isSelected: boolean;
};

export type DestinationEntity = {
  id: string;
  name: string;
  type: string;
  platform?: string;
  isSelected: boolean;
  /**
   * Simulators only: "Booted" or "Shutdown". Undefined for everything else.
   */
  simulatorState?: "Booted" | "Shutdown";
};

export type ConfigurationEntity = {
  name: string;
  isSelected: boolean;
};

export type BuildCommand = "build" | "run" | "launch" | "test" | "clean";
export type BuildStatus = "running" | "succeeded" | "failed" | "cancelled";
export type BuildOriginator = "vscode" | "cli";

export type DiagnosticEntity = {
  file: string;
  line: number;
  column: number;
  severity: "error" | "warning";
  message: string;
};

export type BuildEntity = {
  buildId: string;
  command: BuildCommand;
  scheme: string | null;
  configuration: string | null;
  destination: string | null;
  status: BuildStatus;
  originator: BuildOriginator;
  /** Free-form label set by the caller (CLI `--caller` flag or SWEETPAD_CALLER env). */
  caller: string | null;
  startedAt: string;
  finishedAt: string | null;
  durationMs: number | null;
  errorCount: number;
  warningCount: number;
};

/**
 * One entry returned by `sweetpad servers list`. The active flag is read
 * client-side from active.json so the agent doesn't need a second roundtrip.
 */
export type ServerListEntry = {
  name: string;
  workspacePath: string;
  /**
   * True when this server is the one pointed to by ~/.local/state/sweetpad/active.json.
   * Saves a `cat active.json` round-trip for agents.
   */
  isActive: boolean;
};

/** Aggregate response of `state.get` — one shot of "where are we?". */
export type StateSnapshot = {
  workspacePath: string;
  scheme: SchemeEntity | null;
  destination: DestinationEntity | null;
  configuration: ConfigurationEntity | null;
  running: BuildEntity | null;
  latest: BuildEntity | null;
};

/**
 * Domain-specific error codes carried in `error.data.code`. Stable strings —
 * agents and humans match against these rather than the numeric JSON-RPC code.
 */
export const ERROR_CODES = {
  NO_WORKSPACE: "NO_WORKSPACE",
  SCHEME_NOT_SET: "SCHEME_NOT_SET",
  SCHEME_NOT_FOUND: "SCHEME_NOT_FOUND",
  DESTINATION_NOT_SET: "DESTINATION_NOT_SET",
  DESTINATION_NOT_FOUND: "DESTINATION_NOT_FOUND",
  CONFIG_NOT_SET: "CONFIG_NOT_SET",
  CONFIG_NOT_FOUND: "CONFIG_NOT_FOUND",
  BUILD_NOT_FOUND: "BUILD_NOT_FOUND",
  NO_LAST_BUILD: "NO_LAST_BUILD",
  BUILD_ALREADY_RUNNING: "BUILD_ALREADY_RUNNING",
  WAIT_TIMEOUT: "WAIT_TIMEOUT",
  INVALID_PARAMS: "INVALID_PARAMS",
  MISSING_PREREQUISITES: "MISSING_PREREQUISITES",
  SIMULATOR_NOT_FOUND: "SIMULATOR_NOT_FOUND",
  SIMULATOR_OP_FAILED: "SIMULATOR_OP_FAILED",
  VSCODE_COMMAND_FAILED: "VSCODE_COMMAND_FAILED",
  WORKSPACE_STATE_KEY_INVALID: "WORKSPACE_STATE_KEY_INVALID",
  SETTING_NOT_FOUND: "SETTING_NOT_FOUND",
  BUILD_SETTINGS_FAILED: "BUILD_SETTINGS_FAILED",
  XCODEBUILD_FAILED: "XCODEBUILD_FAILED",
  APP_PATH_NOT_FOUND: "APP_PATH_NOT_FOUND",
  BUNDLE_ID_NOT_FOUND: "BUNDLE_ID_NOT_FOUND",
  SIMCTL_FAILED: "SIMCTL_FAILED",
  DEVICECTL_FAILED: "DEVICECTL_FAILED",
  DEVICE_NOT_FOUND: "DEVICE_NOT_FOUND",
  SCHEME_FILE_NOT_FOUND: "SCHEME_FILE_NOT_FOUND",
  SCHEME_FILE_WRITE_FAILED: "SCHEME_FILE_WRITE_FAILED",
  WORKSPACE_NOT_FOUND: "WORKSPACE_NOT_FOUND",
} as const;
export type ErrorCode = (typeof ERROR_CODES)[keyof typeof ERROR_CODES];
