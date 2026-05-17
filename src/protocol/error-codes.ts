/**
 * Closed error-code enum exchanged between the CLI and server. See
 * `docs/dev/agent-cli.md` §6. Stable across schema revisions — adding a code is
 * additive, renaming one is a breaking change.
 */
export const ERROR_CODES = [
  // Workspace / setup
  "WORKSPACE_NOT_DETECTED",
  "WORKSPACE_LOCKED",

  // Server lifecycle
  "SERVER_UNREACHABLE",
  "SERVER_VERSION_MISMATCH",
  "SERVER_START_FAILED",

  // Scheme
  "SCHEME_NOT_FOUND",
  "SCHEME_AMBIGUOUS",
  "NO_SCHEME_SELECTED",

  // Destination
  "DESTINATION_NOT_FOUND",
  "DESTINATION_AMBIGUOUS",
  "DESTINATION_UNAVAILABLE",
  "NO_DESTINATION_SELECTED",

  // Configuration
  "CONFIG_NOT_FOUND",

  // Build lifecycle
  "BUILD_NOT_FOUND",
  "BUILD_AMBIGUOUS",
  "BUILD_IN_PROGRESS",
  "BUILD_NOT_RUNNING",
  "BUILD_FAILED",
  "BUILD_CANCELLED",

  // Generic
  "INVALID_ARGUMENT",
  "INTERNAL",
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];
