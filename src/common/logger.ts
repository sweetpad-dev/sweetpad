import * as vscode from "vscode";
import { errorReporting } from "./error-reporting";

interface Context {
  message?: never;
  type?: never;
  time?: never;
  [key: string]: unknown;
}

enum LogLevel {
  debug = 0,
  info = 1,
  warning = 2,
  error = 3,
}

interface Message {
  message: string;
  level: LogLevel;
  time: string;
  [key: string]: unknown;
}

/**
 * Logger is a wrapper around vscode.OutputChannel that provides a simple way to
 * log messages to the SweetPad output channel
 */
export class Logger {
  private outputChannel: vscode.OutputChannel;
  private messages: Message[];
  private maxMessages: number;

  // Log level is global for all loggers in the extension
  static level: LogLevel = LogLevel.info;

  constructor(options: { name: string }) {
    this.outputChannel = vscode.window.createOutputChannel(`SweetPad: ${options.name}`);
    this.messages = [];
    this.maxMessages = 1000;
  }

  /**
   * Formats a log message as a YAML-like block:
   *
   * ---
   * time: 2024-12-31T23:59:59.000Z
   * level: INFO
   * message: "Something happened"
   * stackTrace: |
   *   Error: Some error
   *     at someFile.js:123
   * context:
   *   foo: "bar"
   *   count: 42
   */
  private format(data: Message): string {
    const levelName = LogLevel[data.level].toUpperCase();
    const { message, level, time, stackTrace, ...context } = data;

    const lines: string[] = ["---"];
    lines.push(`time: ${time}`);
    lines.push(`level: ${levelName}`);
    lines.push(`message: "${message}"`);

    if (stackTrace && typeof stackTrace === "string" && stackTrace.trim().length > 0) {
      lines.push("stackTrace: |");
      for (const stackLine of stackTrace.split("\n")) {
        lines.push(`  ${stackLine}`);
      }
    }

    if (Object.keys(context).length > 0) {
      lines.push("context:");
      for (const [key, value] of Object.entries(context)) {
        lines.push(`  ${key}: ${this.formatValue(value)}`);
      }
    }

    return lines.join("\n");
  }

  /**
   * Converts a value to a YAML-friendly string without escaping double quotes.
   */
  private formatValue(value: unknown): string {
    if (value === null || value === undefined) {
      return "null";
    }

    if (typeof value === "object") {
      try {
        const str = JSON.stringify(value);
        return `${str}`;
      } catch {
        return `${String(value)}`;
      }
    }

    if (typeof value === "string") {
      return `${value}`;
    }

    return String(value);
  }

  private addMessage(data: Message) {
    if (!this.isOKLevel(data.level)) {
      return;
    }

    const formatted = this.format(data);
    this.outputChannel.appendLine(formatted);
    this.messages.push(data);
    if (this.messages.length >= this.maxMessages) {
      this.messages.shift();
    }
    errorReporting.addBreadcrumb({
      message: data.message,
      category: "log",
      data: data,
    });
  }

  private getNow() {
    return new Date().toISOString();
  }

  private isOKLevel(level: LogLevel) {
    return level >= Logger.level;
  }

  static getLevelFromString(level: string): LogLevel {
    switch (level) {
      case "debug":
        return LogLevel.debug;
      case "info":
        return LogLevel.info;
      case "warning":
        return LogLevel.warning;
      case "error":
        return LogLevel.error;
      default:
        return LogLevel.info;
    }
  }

  static setLevel(level: LogLevel | string) {
    Logger.level = typeof level === "string" ? Logger.getLevelFromString(level) : level;
  }

  static setup() {
    Logger.setLevel(vscode.workspace.getConfiguration("sweetpad").get<string>("system.logLevel") ?? "info");
  }

  debug(message: string, context: Context = {}) {
    this.addMessage({
      message: message,
      level: LogLevel.debug,
      time: this.getNow(),
      ...context,
    });
  }

  log(message: string, context: Context = {}) {
    this.addMessage({
      message: message,
      level: LogLevel.info,
      time: this.getNow(),
      ...context,
    });
  }

  error(message: string, context: Context & { error?: unknown } = {}) {
    const stackTrace = context.error instanceof Error ? (context.error.stack ?? "") : "";
    const { error, ...restContext } = context;
    this.addMessage({
      message: message,
      level: LogLevel.error,
      time: this.getNow(),
      stackTrace: stackTrace,
      ...restContext,
    });
  }

  warn(message: string, context: Context = {}) {
    this.addMessage({
      message: message,
      level: LogLevel.warning,
      time: this.getNow(),
      ...context,
    });
  }

  show() {
    this.outputChannel.show();
  }

  last(n: number): Message[] {
    return this.messages.slice(-n);
  }

  lastFormatted(n: number): string {
    return this.messages.slice(-n).map(this.format.bind(this)).join("\n");
  }
}

export const commonLogger = new Logger({ name: "Common" });
