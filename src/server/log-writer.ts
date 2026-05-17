import * as fs from "node:fs";
import * as path from "node:path";

import type { Logger } from "../core/logger/types";

/**
 * Append-mode writer for a build's `log.txt`. Each call appends `\n` so the
 * file is line-oriented (matches how `LogsGetMethod` returns it). Errors are
 * swallowed and logged — the log file is best-effort; a failed write must
 * never break the build.
 *
 * Use `await using` so the file handle is released even if the build
 * throws.
 */
export class LogWriter implements AsyncDisposable {
  private stream: fs.WriteStream | undefined;
  private constructor(
    private readonly logger: Logger,
    private readonly logPath: string,
  ) {}

  static open(deps: { logger: Logger; logPath: string }): LogWriter {
    const writer = new LogWriter(deps.logger, deps.logPath);
    try {
      fs.mkdirSync(path.dirname(deps.logPath), { recursive: true });
      writer.stream = fs.createWriteStream(deps.logPath, { flags: "w", encoding: "utf8" });
      writer.stream.on("error", (error) => {
        deps.logger.warn("LogWriter stream error", { logPath: deps.logPath, error });
      });
    } catch (error) {
      deps.logger.warn("Failed to open log writer", { logPath: deps.logPath, error });
      writer.stream = undefined;
    }
    return writer;
  }

  write(line: string): void {
    if (!this.stream) return;
    if (!this.stream.write(`${line}\n`)) {
      // Backpressure: write returned false. We keep writing because the
      // build is short-lived enough that draining isn't worth the await
      // plumbing — Node buffers the rest in the meantime.
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  async close(): Promise<void> {
    if (!this.stream) return;
    await new Promise<void>((resolve) => {
      this.stream!.end(() => resolve());
    });
    this.stream = undefined;
  }
}
