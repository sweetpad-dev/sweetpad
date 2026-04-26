import { TERMINAL_COLOR_MAP, type TaskTerminal, type TerminalTextColor } from "../common/tasks/types";

type OsLogNdjsonEntry = {
  timestamp?: string;
  messageType?: string;
  subsystem?: string;
  category?: string;
  eventMessage?: string;
};

const ESC = String.fromCharCode(0x1b);

// Apple ndjson messageType. Default/Notice both map to N — indistinguishable
// at the os_log layer.
export const LEVEL_LETTER: Record<string, string> = {
  Debug: "D",
  Info: "I",
  Default: "N",
  Notice: "N",
  Error: "E",
  Fault: "F",
};

export const LEVEL_COLOR: Record<string, TerminalTextColor> = {
  Debug: "gray",
  Info: "cyan",
  Default: "blue",
  Notice: "blue",
  Error: "red",
  Fault: "magenta",
};

const STRUCTURED_LINE_COLOR: Record<"N" | "I" | "W" | "E", TerminalTextColor> = {
  N: "blue",
  I: "cyan",
  W: "yellow",
  E: "red",
};

const TIMESTAMP_REGEXP = /^\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2})\.(\d{3})/;

// Defensive — upstream checks isatty and we pass --no-color, but strip anyway.
// biome-ignore lint/suspicious/noControlCharactersInRegex: matches ANSI escapes
export const ANSI_ESCAPE_RE = /\x1b\[[0-9;]*m/g;

/** "HH:MM:SS.sss L [cat]" prefix in SGR-1-bold + color, reset at end. */
export function formatLogPrefix(
  time: string,
  level: string,
  category: string,
  color: keyof typeof TERMINAL_COLOR_MAP | undefined,
): string {
  const sgr = color ? `1;${TERMINAL_COLOR_MAP[color]}` : "1";
  return `${ESC}[${sgr}m${time} ${level} [${category}]${ESC}[0m`;
}

function pad(n: number, len: number): string {
  return n.toString().padStart(len, "0");
}

function nowClockTime(): string {
  const d = new Date();
  return `${pad(d.getHours(), 2)}:${pad(d.getMinutes(), 2)}:${pad(d.getSeconds(), 2)}.${pad(d.getMilliseconds(), 3)}`;
}

export function extractClockTime(timestamp: string): string {
  const m = TIMESTAMP_REGEXP.exec(timestamp);
  if (!m) return timestamp;
  return `${m[1]}.${m[2]}`;
}

export function writeStructuredLineAt(
  terminal: TaskTerminal,
  time: string,
  level: "N" | "I" | "W" | "E",
  category: string,
  message: string,
): void {
  const prefix = formatLogPrefix(time, level, category, STRUCTURED_LINE_COLOR[level]);
  terminal.write(`${prefix} ${message}`, { newLine: true });
}

export function writeStructuredLine(
  terminal: TaskTerminal,
  level: "N" | "I" | "W" | "E",
  category: string,
  message: string,
): void {
  writeStructuredLineAt(terminal, nowClockTime(), level, category, message);
}

export function writeInfoLine(terminal: TaskTerminal, category: string, message: string): void {
  writeStructuredLine(terminal, "N", category, message);
}

export function writeWarningLine(terminal: TaskTerminal, category: string, message: string): void {
  writeStructuredLine(terminal, "W", category, message);
}

export function writeErrorLine(terminal: TaskTerminal, category: string, message: string): void {
  writeStructuredLine(terminal, "E", category, message);
}

/** Format one ndjson entry from `simctl spawn ... log stream` / `log stream`. */
export function renderNdjsonLine(line: string, terminal: TaskTerminal): void {
  let entry: OsLogNdjsonEntry;
  try {
    entry = JSON.parse(line);
  } catch {
    writeInfoLine(terminal, "system", line);
    return;
  }
  const msg = entry.eventMessage ?? "";
  const msgType = entry.messageType ?? "Default";
  const letter = LEVEL_LETTER[msgType] ?? "?";
  const color = LEVEL_COLOR[msgType] ?? "gray";
  const time = entry.timestamp ? extractClockTime(entry.timestamp) : nowClockTime();
  const cat = entry.category ?? "?";
  const prefix = formatLogPrefix(time, letter, cat, color);
  terminal.write(`${prefix} ${msg}`, { newLine: true });
}
