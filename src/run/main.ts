import { assertCleanExit } from "../common/tasks/exit";
import type { ProcessGroup, ProcessHandle, ProcessSpec, TaskTerminal } from "../common/tasks/types";
import { ANSI_ESCAPE_RE, writeStructuredLine, writeStructuredLineAt } from "./utils";

// Apple stderr format: NSLog on simulator/macOS, and the os_log mirror under
// OS_ACTIVITY_DT_MODE=enable on device. Capture HH:MM:SS.mmm + body.
// devicectl --console includes a "+ZZZZ" timezone; macOS/simulator don't.
const APPLE_STDERR_RE = /^\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}\.\d{3})\d*(?:[+-]\d{4})? \S+\[\d+:[0-9a-fA-F]+\] (.*)$/;

// First "[category] " after the process prefix. For os_log/Logger it's the
// Swift category; for NSLog it's whatever the caller prefixed to their string.
const LEADING_CATEGORY_RE = /^\[([^\]]+)\] (.*)$/;

/**
 * The foreground app process inside a `runGroup`. Spawns on construction, parses Apple-format
 * stderr (NSLog on simulator/macOS, and the os_log mirror under `OS_ACTIVITY_DT_MODE=enable`
 * on device) line-by-line, and `wait()` resolves once the process exits cleanly (throws on
 * SIGINT or non-zero).
 */
export class MainExecutable {
  pending = "";
  handle: ProcessHandle;
  terminal: TaskTerminal;

  constructor(
    group: ProcessGroup,
    readonly spec: ProcessSpec,
  ) {
    this.terminal = group.terminal;
    // main: true routes terminal input (incl. Ctrl+C) to this child's pty so SIGINT
    // reaches the app via its controlling tty. Sidecars are torn down by cleanupGroup
    // once main exits and runGroup's callback returns.
    this.handle = group.spawn({ ...spec, pty: true, main: true });
    this.handle.onData(this.onData.bind(this));
  }

  async wait(): Promise<void> {
    assertCleanExit(await this.handle.exit, this.spec.command);
  }

  onData(chunk: string): void {
    this.pending += chunk.replace(ANSI_ESCAPE_RE, "");
    let idx = this.pending.indexOf("\n");
    while (idx !== -1) {
      const line = this.pending.slice(0, idx).replace(/\r$/, "");
      this.pending = this.pending.slice(idx + 1);
      this.processStderrLine(line);
      idx = this.pending.indexOf("\n");
    }
  }

  processStderrLine(line: string): void {
    const appleMatch = APPLE_STDERR_RE.exec(line);
    if (appleMatch) {
      const [, time, body] = appleMatch;
      const categoryMatch = LEADING_CATEGORY_RE.exec(body);
      const category = categoryMatch ? categoryMatch[1] : "print";
      const message = categoryMatch ? categoryMatch[2] : body;
      writeStructuredLineAt(this.terminal, time, "N", category, message);
      return;
    }
    writeStructuredLine(this.terminal, "N", "print", line);
  }
}
