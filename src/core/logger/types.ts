export interface LogContext {
  message?: never;
  type?: never;
  time?: never;
  [key: string]: unknown;
}

export interface LogErrorContext extends LogContext {
  error?: unknown;
}

export type LogMessage = {
  message: string;
  level: "debug" | "info" | "warning" | "error";
  time: string;
  [key: string]: unknown;
};

export interface Logger {
  debug(message: string, context?: LogContext): void;
  log(message: string, context?: LogContext): void;
  warn(message: string, context?: LogContext): void;
  error(message: string, context?: LogErrorContext): void;
}

export const noopLogger: Logger = {
  debug() {},
  log() {},
  warn() {},
  error() {},
};
