import type { ErrorCode } from "../protocol/error-codes";

/**
 * Maps an error-code envelope to a CLI process exit code per
 * `docs/dev/agent-cli.md` §5:
 *  - 1: transient / server-side / build-actually-failed (retryable in
 *       principle, agent reads `error.message` for context)
 *  - 2: user error — invalid flag, ambiguous identifier, missing scheme.
 *       Agent shouldn't blindly retry the same invocation.
 */
const USER_ERRORS: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
  "INVALID_ARGUMENT",
  "WORKSPACE_NOT_DETECTED",
  "WORKSPACE_LOCKED",
  "SCHEME_NOT_FOUND",
  "SCHEME_AMBIGUOUS",
  "NO_SCHEME_SELECTED",
  "DESTINATION_NOT_FOUND",
  "DESTINATION_AMBIGUOUS",
  "DESTINATION_UNAVAILABLE",
  "NO_DESTINATION_SELECTED",
  "CONFIG_NOT_FOUND",
  "BUILD_NOT_FOUND",
  "BUILD_AMBIGUOUS",
]);

export function exitCodeForErrorCode(code: ErrorCode): number {
  return USER_ERRORS.has(code) ? 2 : 1;
}
