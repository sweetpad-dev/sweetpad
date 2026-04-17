/**
 * Parse pymobiledevice3 `syslog live --label` output into structured entries and
 * apply local filters on those structured fields.
 *
 * Why structured?
 *   The CLI's `--match`/`--regex` flags filter on the rendered line, which is
 *   brittle in two ways (see PR #231):
 *     1. A `--match <bundleId>` still admits framework-originated log lines
 *        whose subsystem contains the bundle id (e.g. image_name=CoreFoundation
 *        but subsystem=com.example.app → the app's UserDefaults chatter).
 *     2. Apps that use a custom `Logger(subsystem: "My Name")` won't have the
 *        bundle id anywhere on the line, so `--match <bundleId>` drops them.
 *
 *   Both disappear when we filter by `image_name`, which identifies the Mach-O
 *   image that emitted the log. We can't do that at the CLI (no `--image-name`
 *   flag), but we can do it locally on the parsed entry.
 *
 * Why text-parse when a JSON flag could ship upstream?
 *   The CLI format today is hard-coded:
 *     `{timestamp} {process}{{image}{+0xoffset?}}}[{pid}] <{LEVEL}>: {msg}[subsystem][category]?`
 *     (pymobiledevice3/cli/syslog.py::format_line)
 *   If/when upstream adds `--format json`, the Python `SyslogEntry` dataclass
 *   (pymobiledevice3/services/os_trace.py) already maps 1:1 onto `SyslogEntry`
 *   below. Only the parser swaps — the filter and renderer keep working.
 */

export type SyslogLevel = "NOTICE" | "INFO" | "DEBUG" | "USER_ACTION" | "ERROR" | "FAULT" | string;

export type SyslogLabel = {
  subsystem: string;
  category: string;
};

/**
 * Structured log entry.
 *
 * Field names mirror the upstream `pymobiledevice3.services.os_trace.SyslogEntry`
 * dataclass so a future `--format json` mode can drop straight into this shape:
 *   pid, timestamp, level, image_name, image_offset, filename, message, label.
 * (We use camelCase in TypeScript; `filename` / `processName` are the path and
 * its basename respectively — upstream `SyslogEntry` only carries `filename`.)
 */
export type SyslogEntry = {
  timestamp: string;
  processName: string;
  imageName: string;
  imageOffset?: number;
  pid: number;
  level: SyslogLevel;
  message: string;
  label?: SyslogLabel;
};

// Strip ANSI color escape sequences. pymobiledevice3 may emit them even when
// stdout is piped, depending on the version and whether `--no-color` is set.
const ANSI_ESCAPE_RE = /\x1b\[[0-9;]*m/g;

// Matches `format_line()` in pymobiledevice3/cli/syslog.py:
//   "{timestamp} {process_name}{{{image_name}{image_offset_str}}}[{pid}] <{level}>: {message}"
// image_offset_str is empty by default or "+0x{hex}" with --image-offset.
const LINE_RE =
  /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+) (.+?)\{([^}]+?)(?:\+0x([0-9a-fA-F]+))?\}\[(\d+)\] <([^>]*)>: (.*)$/;

// When --label is passed, the line ends with " [subsystem][category]".
// Anchored to the end so that "][" inside the message body isn't consumed.
const LABEL_SUFFIX_RE = / \[([^\]]*)\]\[([^\]]*)\]$/;

/**
 * Parse a single line of `pymobiledevice3 syslog live --label` output into a
 * {@link SyslogEntry}, or `null` if the line doesn't match the known format
 * (e.g. a tunnel notice, a blank line, or a future format change).
 */
export function parseSyslogLine(rawLine: string): SyslogEntry | null {
  const line = rawLine.replace(ANSI_ESCAPE_RE, "").replace(/\r$/, "");
  const match = LINE_RE.exec(line);
  if (!match) {
    return null;
  }
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

export type SyslogFilter = (entry: SyslogEntry) => boolean;

/**
 * Build a filter that keeps only entries emitted by the app's own Mach-O image.
 *
 * When Xcode's `ENABLE_DEBUG_DYLIB` is on, the app's code runs from
 * `${EXECUTABLE_NAME}.debug.dylib` while the thin launcher keeps the original
 * executable name, so both are accepted.
 */
export function buildAppImageFilter(executableName: string): SyslogFilter {
  const debugDylib = `${executableName}.debug.dylib`;
  return (entry) => entry.imageName === executableName || entry.imageName === debugDylib;
}

/**
 * Split a stream of chunked stdout into complete lines.
 *
 * `child_process` emits `data` Buffers that are not line-aligned; this keeps
 * partial fragments between calls and flushes any trailing fragment on close.
 */
export function createLineBuffer(onLine: (line: string) => void): {
  push(chunk: string): void;
  flush(): void;
} {
  let pending = "";
  return {
    push(chunk) {
      pending += chunk;
      let idx = pending.indexOf("\n");
      while (idx !== -1) {
        const line = pending.slice(0, idx);
        pending = pending.slice(idx + 1);
        onLine(line);
        idx = pending.indexOf("\n");
      }
    },
    flush() {
      if (pending.length > 0) {
        onLine(pending);
        pending = "";
      }
    },
  };
}

export type SyslogLineProcessorOptions = {
  /** Keep only entries emitted by the app's own image (incl. `.debug.dylib`). */
  executableName: string;
  /** Optional subsystem allow-list; if set, the entry's subsystem must be in it. */
  subsystems?: readonly string[];
  /** Optional minimum level; entries below this level are dropped. */
  minLevel?: SyslogLevel;
};

const LEVEL_RANK: Record<string, number> = {
  DEBUG: 0,
  INFO: 1,
  NOTICE: 2,
  USER_ACTION: 3,
  ERROR: 4,
  FAULT: 5,
};

/**
 * Decide whether a raw line from `pymobiledevice3 syslog live --label` should
 * be forwarded to the output channel.
 *
 * Returns the line to display (original text), or `null` to drop it. Lines we
 * can't parse pass through so users still see tunnel/diagnostic messages.
 */
export function createSyslogLineProcessor(options: SyslogLineProcessorOptions): (line: string) => string | null {
  const appFilter = buildAppImageFilter(options.executableName);
  const subsystems = options.subsystems && options.subsystems.length > 0 ? new Set(options.subsystems) : undefined;
  const minRank = options.minLevel !== undefined ? LEVEL_RANK[options.minLevel] : undefined;

  return (line) => {
    if (line.length === 0) {
      return null;
    }
    const entry = parseSyslogLine(line);
    if (!entry) {
      return line;
    }
    if (!appFilter(entry)) {
      return null;
    }
    if (subsystems && (entry.label === undefined || !subsystems.has(entry.label.subsystem))) {
      return null;
    }
    if (minRank !== undefined) {
      const rank = LEVEL_RANK[entry.level];
      if (rank !== undefined && rank < minRank) {
        return null;
      }
    }
    return line;
  };
}
