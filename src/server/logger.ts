import type { LogContext, LogErrorContext, Logger } from "../core/logger/types";

const LEVELS: Record<string, number> = { debug: 0, info: 1, warning: 2, error: 3 };

/**
 * Stderr-backed logger for the server process. Each line is a JSON object so
 * `sweetpad-server` output is consumable both by humans following the process
 * and (eventually) by a log shipper. Stdout is reserved for the wire protocol
 * exclusively in the CLI; the server has no protocol output on stdout.
 */
export class StderrJsonLogger implements Logger {
  constructor(private readonly minLevel: keyof typeof LEVELS = "info") {}

  debug(message: string, context?: LogContext): void {
    this.emit("debug", message, context);
  }

  log(message: string, context?: LogContext): void {
    this.emit("info", message, context);
  }

  warn(message: string, context?: LogContext): void {
    this.emit("warning", message, context);
  }

  error(message: string, context?: LogErrorContext): void {
    this.emit("error", message, context);
  }

  private emit(level: keyof typeof LEVELS, message: string, context?: LogContext): void {
    if (LEVELS[level] < LEVELS[this.minLevel]) return;
    const line = {
      time: new Date().toISOString(),
      level,
      message,
      ...(context ?? {}),
    };
    process.stderr.write(`${JSON.stringify(line, replaceErrors)}\n`);
  }
}

function replaceErrors(_key: string, value: unknown): unknown {
  if (value instanceof Error) {
    return { name: value.name, message: value.message, stack: value.stack };
  }
  return value;
}
