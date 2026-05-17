import type { ErrorCode } from "./error-codes";
import type { WireErrorPayload } from "./types";

/**
 * Thrown by server method handlers to short-circuit with a structured error.
 * The dispatcher catches it and wraps it in the envelope. CLI also throws this
 * for argv-validation failures before any socket call.
 */
export class ProtocolError extends Error {
  public readonly code: ErrorCode;
  public readonly hint: string | undefined;
  public readonly extra: Record<string, unknown> | undefined;

  constructor(code: ErrorCode, message: string, options?: { hint?: string; extra?: Record<string, unknown> }) {
    super(message);
    this.code = code;
    this.hint = options?.hint;
    this.extra = options?.extra;
  }

  toPayload(): WireErrorPayload {
    return {
      code: this.code,
      message: this.message,
      ...(this.hint !== undefined ? { hint: this.hint } : {}),
    };
  }
}
