/**
 * Shared wire-protocol types. Imported by both the extension server and the
 * CLI client — must not pull in `vscode` or any extension-only module.
 */

export const PROTOCOL_VERSION = "1.0";

// JSON-RPC error codes SweetPad emits. INVALID_PARAMS (-32602) and
// INTERNAL_ERROR (-32603) are reserved codes; every other failure uses the
// application-defined -32000 and carries a stable string code in error.data.
export const JSON_RPC_INVALID_PARAMS = -32602;
export const JSON_RPC_INTERNAL_ERROR = -32603;
export const SWEETPAD_APPLICATION_ERROR = -32000;

// The `<workspace>/.sweetpad/cli.json` connection file: enough to find and
// connect to the running CLI control server. `socket` is the short tmpdir path
// the server bound (see `getSocketPath`). Last-writer-wins across windows.
export type CliServerMetadata = {
  name: string;
  socket: string;
  workspacePath: string;
  pid: number;
  startedAt: string;
  extensionVersion: string;
  protocolVersion: string;
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
  WORKSPACE_NOT_FOUND: "WORKSPACE_NOT_FOUND",
} as const;
export type ErrorCode = (typeof ERROR_CODES)[keyof typeof ERROR_CODES];
