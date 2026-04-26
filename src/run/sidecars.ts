import { quote } from "shell-quote";
import { getWorkspaceConfig } from "../common/config";
import { exec } from "../common/exec";
import { commonLogger } from "../common/logger";
import type { ProcessGroup, ProcessSpec, TaskTerminal } from "../common/tasks/types";
import {
  ANSI_ESCAPE_RE,
  extractClockTime,
  formatLogPrefix,
  LEVEL_COLOR,
  LEVEL_LETTER,
  renderNdjsonLine,
  writeErrorLine,
  writeInfoLine,
  writeWarningLine,
} from "./utils";

type SyslogLevel = "NOTICE" | "INFO" | "DEBUG" | "USER_ACTION" | "ERROR" | "FAULT" | string;

type SyslogLabel = {
  subsystem: string;
  category: string;
};

/** Mirrors upstream `pymobiledevice3.services.os_trace.SyslogEntry`. */
type SyslogEntry = {
  timestamp: string;
  processName: string;
  // Mach-O binary that emitted the log; "CoreFoundation" = framework, "X.debug.dylib" = the app.
  imageName: string;
  imageOffset?: number;
  pid: number;
  level: SyslogLevel;
  message: string;
  label?: SyslogLabel;
};

type Pymobiledevice3FilterOptions = {
  executableName: string;
  debugDylibOnly?: boolean;
  subsystemDenyList?: readonly string[];
  subsystemAllowList?: readonly string[];
  minLevel?: SyslogLevel;
};

type Pymobiledevice3ArgsResult =
  | { kind: "ok"; args: string[]; hasProcessNameOverride: boolean }
  | { kind: "missingProcessName" };

// USER_ACTION (iOS-only user-interaction event) folds into N so both backends
// share the same letter/color tables.
const PYMOBILEDEVICE3_LEVEL_TO_NDJSON: Record<string, string> = {
  DEBUG: "Debug",
  INFO: "Info",
  NOTICE: "Notice",
  USER_ACTION: "Notice",
  ERROR: "Error",
  FAULT: "Fault",
};

// "2026-04-16 12:52:32.707333 Laboratory{Laboratory.debug.dylib+0x1a2b}[67135] <NOTICE>: msg"
const SYSLOG_LINE_RE =
  /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+) (.+?)\{([^}]+?)(?:\+0x([0-9a-fA-F]+))?\}\[(\d+)\] <([^>]*)>: (.*)$/;

// Trailing "[subsystem][category]" that --label appends.
const SYSLOG_LABEL_SUFFIX_RE = / \[([^\]]*)\]\[([^\]]*)\]$/;

const SYSLOG_LEVEL_RANK: Record<string, number> = {
  DEBUG: 0,
  INFO: 1,
  NOTICE: 2,
  USER_ACTION: 3,
  ERROR: 4,
  FAULT: 5,
};

abstract class LogSidecar {
  stdoutPending = "";
  stderrPending = "";

  constructor(readonly group: ProcessGroup) {}

  get terminal(): TaskTerminal {
    return this.group.terminal;
  }

  abstract spec(): Promise<ProcessSpec | null>;
  abstract processStdoutLine(line: string): void;

  /** Default: write each non-empty line as an error in the [system] category. */
  processStderrLine(line: string): void {
    if (line.length === 0) return;
    writeErrorLine(this.terminal, "system", line);
  }

  isLogStreamEnabled(): boolean {
    return getWorkspaceConfig("build.logStreamEnabled") ?? true;
  }

  /**
   * Builds the `log stream` predicate that selects which os_log/Logger entries reach the terminal.
   *
   * `bundleId` is the app's bundle identifier (e.g. "com.example.MyApp") and `executableName`
   * is CFBundleExecutable — the process name as it appears in os_log (e.g. "MyApp").
   *
   * The default matches by image (process + sender) rather than by subsystem, so apps that don't
   * set `Logger(subsystem:)` still show up while Apple framework chatter is filtered out. Both
   * the bare executable and its `.debug.dylib` sidecar are accepted to cover Xcode 15+ Debug
   * Dylib Support, which loads app code from a separate dylib in Debug builds.
   *
   * Users can override the whole predicate via `build.logStreamPredicate`, with `${bundleId}` and
   * `${processName}` placeholders.
   */
  buildPredicate(bundleId: string, executableName: string): string {
    const custom = getWorkspaceConfig("build.logStreamPredicate");
    if (custom) {
      return custom.replace(/\$\{bundleId\}/g, bundleId).replace(/\$\{processName\}/g, executableName);
    }
    return `process == "${executableName}" AND (sender == "${executableName}" OR sender == "${executableName}.debug.dylib")`;
  }

  async spawn(): Promise<void> {
    const spec = await this.spec();
    if (!spec) return;
    const handle = this.group.spawn(spec);
    handle.onData(this.onStdout.bind(this));
    handle.onError(this.onStderr.bind(this));
  }

  onStdout(chunk: string): void {
    this.stdoutPending += chunk.replace(ANSI_ESCAPE_RE, "");
    let idx = this.stdoutPending.indexOf("\n");
    while (idx !== -1) {
      const line = this.stdoutPending.slice(0, idx).replace(/\r$/, "");
      this.stdoutPending = this.stdoutPending.slice(idx + 1);
      this.processStdoutLine(line);
      idx = this.stdoutPending.indexOf("\n");
    }
  }

  onStderr(chunk: string): void {
    this.stderrPending += chunk.replace(ANSI_ESCAPE_RE, "");
    let idx = this.stderrPending.indexOf("\n");
    while (idx !== -1) {
      const line = this.stderrPending.slice(0, idx).replace(/\r$/, "");
      this.stderrPending = this.stderrPending.slice(idx + 1);
      this.processStderrLine(line);
      idx = this.stderrPending.indexOf("\n");
    }
  }
}

export type MacOSLogSidecarOptions = {
  bundleId: string;
  executableName: string;
};

export class MacOSLogSidecar extends LogSidecar {
  constructor(
    group: ProcessGroup,
    readonly options: MacOSLogSidecarOptions,
  ) {
    super(group);
  }

  async spec(): Promise<ProcessSpec | null> {
    if (!this.isLogStreamEnabled()) return null;
    return {
      command: "log",
      args: [
        "stream",
        "--predicate",
        this.buildPredicate(this.options.bundleId, this.options.executableName),
        "--level",
        "debug",
        "--style",
        "ndjson",
      ],
    };
  }

  processStdoutLine(line: string): void {
    renderNdjsonLine(line, this.terminal);
  }
}

export type SimulatorLogSidecarOptions = {
  simulatorUdid: string;
  bundleId: string;
  executableName: string;
};

export class SimulatorLogSidecar extends LogSidecar {
  constructor(
    group: ProcessGroup,
    readonly options: SimulatorLogSidecarOptions,
  ) {
    super(group);
  }

  async spec(): Promise<ProcessSpec | null> {
    if (!this.isLogStreamEnabled()) return null;
    return {
      command: "xcrun",
      args: [
        "simctl",
        "spawn",
        this.options.simulatorUdid,
        "log",
        "stream",
        "--predicate",
        this.buildPredicate(this.options.bundleId, this.options.executableName),
        "--level",
        "debug",
        "--style",
        "ndjson",
      ],
    };
  }

  processStdoutLine(line: string): void {
    renderNdjsonLine(line, this.terminal);
  }
}

export type Pymd3SidecarOptions = {
  executableName: string | undefined;
};

export class Pymd3Sidecar extends LogSidecar {
  readonly rawExtraArgs: (string | null)[];
  readonly filter: ((entry: SyslogEntry) => boolean) | null;
  // Starts true so pre-entry banners (tunnel notices) pass through.
  keepPrevious = true;

  constructor(
    group: ProcessGroup,
    readonly options: Pymd3SidecarOptions,
  ) {
    super(group);
    this.rawExtraArgs = getWorkspaceConfig("build.pymobiledevice3ExtraArgs") ?? [];
    const filterExecutable = this.extractProcessNameOverride(this.rawExtraArgs) ?? options.executableName;
    this.filter = filterExecutable
      ? this.buildPymd3Filter({
          executableName: filterExecutable,
          debugDylibOnly: getWorkspaceConfig("build.pymobiledevice3DebugDylibOnly") ?? true,
          subsystemDenyList: getWorkspaceConfig("build.pymobiledevice3SubsystemDenyList") ?? ["com.apple.*"],
          subsystemAllowList: getWorkspaceConfig("build.pymobiledevice3SubsystemAllowList") ?? [],
        })
      : null;
  }

  async spec(): Promise<ProcessSpec | null> {
    if (!this.isLogStreamEnabled()) return null;

    const binaryPath = getWorkspaceConfig("build.pymobiledevice3Path") ?? "pymobiledevice3";
    if (!(await this.isPymobiledevice3Available(binaryPath))) {
      writeWarningLine(
        this.terminal,
        "sweetpad",
        `'${binaryPath}' not found — device os_log/Logger output won't be streamed.`,
      );
      // Plain terminal.write keeps the install hints unprefixed so they read like a
      // help block rather than several repeated [sweetpad] log lines.
      this.terminal.write("  Install pymobiledevice3 to stream logs from physical iOS devices:", {
        newLine: true,
      });
      this.terminal.write("  • Install uv: brew install uv", { newLine: true });
      this.terminal.write("  • Then: uv tool install pymobiledevice3", { newLine: true });
      this.terminal.write("  Run command: Sweetpad: Install pymobiledevice3", { newLine: true });
      return null;
    }

    const args = this.buildPymobiledevice3Args({
      rawExtraArgs: this.rawExtraArgs,
      processName: this.options.executableName,
    });
    if (args.kind === "missingProcessName") {
      writeErrorLine(
        this.terminal,
        "sweetpad",
        "Could not determine process name for log filtering (EXECUTABLE_NAME missing).",
      );
      return null;
    }

    commonLogger.debug("pymobiledevice3 log stream", {
      extraArgs: this.rawExtraArgs,
      command: quote([binaryPath, ...args.args]),
    });

    return { command: binaryPath, args: args.args };
  }

  processStdoutLine(line: string): void {
    if (line.length === 0) return;
    const entry = this.parseSyslogLine(line);
    if (!entry) {
      if (this.keepPrevious) writeInfoLine(this.terminal, "pymobiledevice3", line);
      return;
    }
    this.keepPrevious = this.filter ? this.filter(entry) : true;
    if (!this.keepPrevious) return;
    this.renderEntry(entry);
  }

  processStderrLine(line: string): void {
    if (line.length === 0) return;
    writeErrorLine(this.terminal, "pymobiledevice3", line);
  }

  /** Parse one line emitted by `pymobiledevice3 syslog live --label`. */
  parseSyslogLine(rawLine: string): SyslogEntry | null {
    const line = rawLine.replace(ANSI_ESCAPE_RE, "").replace(/\r$/, "");
    const match = SYSLOG_LINE_RE.exec(line);
    if (!match) {
      return null;
    }
    const [, timestamp, processName, imageName, imageOffsetHex, pidStr, level, rest] = match;

    let message = rest;
    let label: SyslogLabel | undefined;
    const labelMatch = SYSLOG_LABEL_SUFFIX_RE.exec(rest);
    if (labelMatch) {
      message = rest.slice(0, labelMatch.index);
      label = { subsystem: labelMatch[1], category: labelMatch[2] };
    }

    const entry: SyslogEntry = {
      timestamp,
      processName,
      imageName,
      pid: Number.parseInt(pidStr, 10),
      level,
      message,
    };
    if (imageOffsetHex !== undefined) {
      entry.imageOffset = Number.parseInt(imageOffsetHex, 16);
    }
    if (label) {
      entry.label = label;
    }
    return entry;
  }

  renderEntry(entry: SyslogEntry): void {
    const ndjsonType = PYMOBILEDEVICE3_LEVEL_TO_NDJSON[entry.level] ?? "Default";
    const letter = LEVEL_LETTER[ndjsonType] ?? "?";
    const color = LEVEL_COLOR[ndjsonType] ?? "blue";
    const time = extractClockTime(entry.timestamp);
    // Fall back to the image name when the call site didn't set a category.
    const category = entry.label?.category ?? entry.imageName;
    const prefix = formatLogPrefix(time, letter, category, color);
    this.terminal.write(`${prefix} ${entry.message}`, { newLine: true });
  }

  buildPymd3Filter(opts: Pymobiledevice3FilterOptions): (entry: SyslogEntry) => boolean {
    const debugDylib = `${opts.executableName}.debug.dylib`;
    const appFilter = opts.debugDylibOnly
      ? (entry: SyslogEntry) => entry.imageName === debugDylib
      : (entry: SyslogEntry) => entry.imageName === opts.executableName || entry.imageName === debugDylib;
    const denyMatch = this.compilePatterns(opts.subsystemDenyList ?? []);
    const allowMatch = this.compilePatterns(opts.subsystemAllowList ?? []);
    const minRank = opts.minLevel !== undefined ? SYSLOG_LEVEL_RANK[opts.minLevel] : undefined;

    return (entry) => {
      if (!appFilter(entry)) return false;
      const subsystem = entry.label?.subsystem;
      if (denyMatch && subsystem && denyMatch(subsystem)) return false;
      if (allowMatch && (!subsystem || !allowMatch(subsystem))) return false;
      if (minRank !== undefined) {
        const rank = SYSLOG_LEVEL_RANK[entry.level];
        if (rank !== undefined && rank < minRank) return false;
      }
      return true;
    };
  }

  compilePatterns(patterns: readonly string[]): ((value: string) => boolean) | undefined {
    if (patterns.length === 0) return undefined;
    const regexes = patterns.map((p) => this.patternToRegex(p));
    return (value) => regexes.some((re) => re.test(value));
  }

  patternToRegex(pattern: string): RegExp {
    const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
    return new RegExp(`^${escaped.replace(/\*/g, ".*")}$`);
  }

  // `--process-name`/`-p` in extras replaces the default; null after it suppresses the default entirely.
  buildPymobiledevice3Args(input: {
    rawExtraArgs: (string | null)[];
    processName: string | undefined;
  }): Pymobiledevice3ArgsResult {
    const { rawExtraArgs, processName } = input;

    let hasProcessNameOverride = false;
    const cleanedExtra: string[] = [];

    for (let i = 0; i < rawExtraArgs.length; i++) {
      const arg = rawExtraArgs[i];
      const isProcessName = arg === "--process-name" || arg === "-p";
      if (isProcessName) {
        hasProcessNameOverride = true;
        const value = rawExtraArgs[i + 1];
        if (typeof value === "string") {
          cleanedExtra.push(arg as string, value);
        }
        i++;
        continue;
      }
      if (typeof arg === "string") {
        cleanedExtra.push(arg);
      }
    }

    if (!processName && !hasProcessNameOverride) {
      return { kind: "missingProcessName" };
    }

    // `--no-color` is a top-level option and must precede the `syslog` subcommand.
    // `--label` makes the CLI emit `[subsystem][category]` suffixes the parser reads.
    const baseArgs: string[] = ["--no-color", "syslog", "live", "--label"];
    if (!hasProcessNameOverride) {
      baseArgs.push("--process-name", processName as string);
    }

    return {
      kind: "ok",
      args: [...baseArgs, ...cleanedExtra],
      hasProcessNameOverride,
    };
  }

  extractProcessNameOverride(rawExtraArgs: (string | null)[]): string | undefined {
    for (let i = 0; i < rawExtraArgs.length; i++) {
      const arg = rawExtraArgs[i];
      if (arg === "--process-name" || arg === "-p") {
        const value = rawExtraArgs[i + 1];
        if (typeof value === "string") {
          return value;
        }
        return undefined;
      }
    }
    return undefined;
  }

  async isPymobiledevice3Available(binaryPath: string): Promise<boolean> {
    try {
      await exec({ command: binaryPath, args: ["version"] });
      return true;
    } catch {
      return false;
    }
  }
}
