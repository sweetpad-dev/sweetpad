/**
 * Parse pymobiledevice3 `syslog live --label` output into structured entries and
 * apply local filters on those structured fields.
 *
 * Why structured?
 *   The CLI's "--match"/"--regex" flags filter on the rendered line, which is
 *   brittle in two ways (see PR #231):
 *     1. A "--match <bundleId>" still admits framework-originated log lines
 *        whose subsystem contains the bundle id (e.g. image_name=CoreFoundation
 *        but subsystem=com.example.app → the app's UserDefaults chatter).
 *     2. Apps that use a custom `Logger(subsystem: "My Name")` won't have the
 *        bundle id anywhere on the line, so "--match <bundleId>" drops them.
 *
 *   Both disappear when we filter by `image_name`, which identifies the Mach-O
 *   image that emitted the log. We can't do that at the CLI (no "--image-name"
 *   flag), but we can do it locally on the parsed entry.
 *
 * Why text-parse when a JSON flag could ship upstream?
 *   The CLI format today is hard-coded:
 *     `{timestamp} {process}{{image}{+0xoffset?}}}[{pid}] <{LEVEL}>: {msg}[subsystem][category]?`
 *     (pymobiledevice3/cli/syslog.py::format_line)
 *   If/when upstream adds "--format json", the Python `SyslogEntry` dataclass
 *   (pymobiledevice3/services/os_trace.py) already maps 1:1 onto `SyslogEntry`
 *   below. Only the parser swaps — the filter and renderer keep working.
 */

const LEVEL_RANK: Record<string, number> = {
  DEBUG: 0,
  INFO: 1,
  NOTICE: 2,
  USER_ACTION: 3,
  ERROR: 4,
  FAULT: 5,
};

// Defensive ANSI stripping — shouldn't appear (upstream checks isatty, we pass --no-color).
// biome-ignore lint/suspicious/noControlCharactersInRegex: intentional — matching ANSI escape sequences
const ANSI_ESCAPE_RE = /\x1b\[[0-9;]*m/g;

// Example: "2026-04-16 12:52:32.707333 Laboratory{Laboratory.debug.dylib+0x1a2b}[67135] <NOTICE>: hello"
//           |---- timestamp ---------| |process| |--- image + offset? ---| |pid|  |level|  |msg|
const LINE_RE =
  /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+) (.+?)\{([^}]+?)(?:\+0x([0-9a-fA-F]+))?\}\[(\d+)\] <([^>]*)>: (.*)$/;

// Example: "... some message [com.example.app][Network]"
//                             |-- subsystem --||category|
const LABEL_SUFFIX_RE = / \[([^\]]*)\]\[([^\]]*)\]$/;

export type SyslogLevel = "NOTICE" | "INFO" | "DEBUG" | "USER_ACTION" | "ERROR" | "FAULT" | string;

export type SyslogLabel = {
  subsystem: string;
  category: string;
};

/** Mirrors upstream `pymobiledevice3.services.os_trace.SyslogEntry`. */
export type SyslogEntry = {
  timestamp: string;
  processName: string;
  // "Image" is macOS/Mach-O terminology for any executable binary loaded into a
  // process: the main app binary, dynamic libraries (.dylib), or frameworks.
  // Apple's unified logging stores the image_name of the Mach-O binary that
  // called the log function. For example, "CoreFoundation" means the log was
  // emitted by the CoreFoundation framework, while "Laboratory.debug.dylib"
  // means it came from the app's own code.
  imageName: string;
  imageOffset?: number;
  pid: number;
  level: SyslogLevel;
  message: string;
  label?: SyslogLabel;
};

export type SyslogFilter = (entry: SyslogEntry) => boolean;

export type LineOutput = (text: string) => void;

/** Processes raw stdout chunks and writes to the output. */
export interface LogPipe {
  push(chunk: string): void;
  flush(): void;
}

export type Pymobiledevice3LogPipeOptions = {
  executableName: string;
  debugDylibOnly?: boolean;
  subsystemDenyList?: readonly string[];
  subsystemAllowList?: readonly string[];
  minLevel?: SyslogLevel;
};

type Pymobiledevice3JsonEntry = {
  pid: number;
  timestamp: string;
  level: string;
  image_name: string;
  image_offset: number;
  filename: string;
  message: string;
  label: { subsystem: string; category: string } | null;
};

// Convert a simple glob pattern (e.g. "com.apple.*") into a RegExp.
// Only "*" is a wildcard (matches zero or more characters).
function patternToRegex(pattern: string): RegExp {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`^${escaped.replace(/\*/g, ".*")}$`);
}

// Compile an array of glob patterns into a single predicate, or undefined if empty.
function compilePatterns(patterns: readonly string[]): ((value: string) => boolean) | undefined {
  if (patterns.length === 0) return undefined;
  const regexes = patterns.map(patternToRegex);
  return (value) => regexes.some((re) => re.test(value));
}

export function parseSyslogLine(rawLine: string): SyslogEntry | null {
  const line = rawLine.replace(ANSI_ESCAPE_RE, "").replace(/\r$/, "");
  const match = LINE_RE.exec(line);
  if (!match) {
    return null;
  }
  // imageName: Mach-O binary that emitted the log (e.g. "Laboratory.debug.dylib", "CoreFoundation")
  // imageOffsetHex: instruction offset within that image (only present with --image-offset)
  const [, timestamp, processName, imageName, imageOffsetHex, pidStr, level, rest] = match;

  let message = rest;
  let label: SyslogLabel | undefined;
  const labelMatch = LABEL_SUFFIX_RE.exec(rest);
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

export function buildAppImageFilter(executableName: string, debugDylibOnly = false): SyslogFilter {
  const debugDylib = `${executableName}.debug.dylib`;
  if (debugDylibOnly) {
    return (entry) => entry.imageName === debugDylib;
  }
  return (entry) => entry.imageName === executableName || entry.imageName === debugDylib;
}

function basename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}

/** Forwards raw chunks as-is. */
export class PassthroughLogPipe implements LogPipe {
  constructor(private readonly output: LineOutput) {}

  push(chunk: string): void {
    this.output(chunk);
  }
  flush(): void {}
}

/** Text-mode pipe for `syslog live --label`. Parses lines via regex and filters. */
export class Pymobiledevice3LogPipe implements LogPipe {
  private readonly output: LineOutput;
  private readonly appFilter: SyslogFilter;
  private readonly denyMatch: ((subsystem: string) => boolean) | undefined;
  private readonly allowMatch: ((subsystem: string) => boolean) | undefined;
  private readonly minRank: number | undefined;
  private pending = "";
  // Tracks whether the last parsed entry was kept or dropped.
  // Unparseable lines (continuations of multi-line messages) inherit this decision.
  // Starts as "keep" so lines before the first parsed entry (tunnel notices) pass through.
  private keepPrevious = true;

  constructor(output: LineOutput, options: Pymobiledevice3LogPipeOptions) {
    this.output = output;
    this.appFilter = buildAppImageFilter(options.executableName, options.debugDylibOnly ?? false);
    this.denyMatch = compilePatterns(options.subsystemDenyList ?? []);
    this.allowMatch = compilePatterns(options.subsystemAllowList ?? []);
    this.minRank = options.minLevel !== undefined ? LEVEL_RANK[options.minLevel] : undefined;
  }

  push(chunk: string): void {
    this.pending += chunk;
    let idx = this.pending.indexOf("\n");
    while (idx !== -1) {
      const line = this.pending.slice(0, idx);
      this.pending = this.pending.slice(idx + 1);
      const result = this.processLine(line);
      if (result !== null) {
        this.output(result);
      }
      idx = this.pending.indexOf("\n");
    }
  }

  flush(): void {
    if (this.pending.length > 0) {
      const line = this.pending;
      this.pending = "";
      const result = this.processLine(line);
      if (result !== null) {
        this.output(result);
      }
    }
  }

  private processLine(line: string): string | null {
    if (line.length === 0) {
      return null;
    }
    const entry = parseSyslogLine(line);
    if (!entry) {
      return this.keepPrevious ? line : null;
    }
    if (!this.appFilter(entry)) {
      this.keepPrevious = false;
      return null;
    }
    const subsystem = entry.label?.subsystem;
    if (this.denyMatch && subsystem && this.denyMatch(subsystem)) {
      this.keepPrevious = false;
      return null;
    }
    if (this.allowMatch && (!subsystem || !this.allowMatch(subsystem))) {
      this.keepPrevious = false;
      return null;
    }
    if (this.minRank !== undefined) {
      const rank = LEVEL_RANK[entry.level];
      if (rank !== undefined && rank < this.minRank) {
        this.keepPrevious = false;
        return null;
      }
    }
    this.keepPrevious = true;
    return line;
  }
}

/**
 * JSON-mode pipe for `syslog live --format json` (see doronz88/pymobiledevice3#1659).
 * Not wired up yet.
 */
export class Pymobiledevice3JsonLogPipe implements LogPipe {
  private readonly output: LineOutput;
  private readonly appFilter: SyslogFilter;
  private readonly denyMatch: ((subsystem: string) => boolean) | undefined;
  private readonly allowMatch: ((subsystem: string) => boolean) | undefined;
  private readonly minRank: number | undefined;
  private pending = "";

  constructor(output: LineOutput, options: Pymobiledevice3LogPipeOptions) {
    this.output = output;
    this.appFilter = buildAppImageFilter(options.executableName, options.debugDylibOnly ?? false);
    this.denyMatch = compilePatterns(options.subsystemDenyList ?? []);
    this.allowMatch = compilePatterns(options.subsystemAllowList ?? []);
    this.minRank = options.minLevel !== undefined ? LEVEL_RANK[options.minLevel] : undefined;
  }

  push(chunk: string): void {
    this.pending += chunk;
    let idx = this.pending.indexOf("\n");
    while (idx !== -1) {
      const line = this.pending.slice(0, idx);
      this.pending = this.pending.slice(idx + 1);
      this.processLine(line);
      idx = this.pending.indexOf("\n");
    }
  }

  flush(): void {
    if (this.pending.length > 0) {
      const line = this.pending;
      this.pending = "";
      this.processLine(line);
    }
  }

  private processLine(line: string): void {
    if (line.length === 0) {
      return;
    }
    let json: Pymobiledevice3JsonEntry;
    try {
      json = JSON.parse(line);
    } catch {
      this.output(line);
      return;
    }
    const entry = this.toSyslogEntry(json);
    if (!this.appFilter(entry)) {
      return;
    }
    const subsystem = entry.label?.subsystem;
    if (this.denyMatch && subsystem && this.denyMatch(subsystem)) {
      return;
    }
    if (this.allowMatch && (!subsystem || !this.allowMatch(subsystem))) {
      return;
    }
    if (this.minRank !== undefined) {
      const rank = LEVEL_RANK[entry.level];
      if (rank !== undefined && rank < this.minRank) {
        return;
      }
    }
    this.output(this.formatEntry(entry));
  }

  private toSyslogEntry(json: Pymobiledevice3JsonEntry): SyslogEntry {
    const imageName = basename(json.image_name);
    const entry: SyslogEntry = {
      timestamp: json.timestamp,
      processName: basename(json.filename),
      imageName,
      pid: json.pid,
      level: json.level,
      message: json.message,
    };
    if (json.image_offset !== 0) {
      entry.imageOffset = json.image_offset;
    }
    if (json.label !== null) {
      entry.label = json.label;
    }
    return entry;
  }

  private formatEntry(entry: SyslogEntry): string {
    const label = entry.label ? ` [${entry.label.subsystem}][${entry.label.category}]` : "";
    return `${entry.timestamp} ${entry.processName}{${entry.imageName}}[${entry.pid}] <${entry.level}>: ${entry.message}${label}`;
  }
}
