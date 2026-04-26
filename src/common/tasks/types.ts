export type TaskExecutor = "v2" | "v3";

export type Command = {
  command: string;
  args?: string[];
};

export type CommandOptions = {
  command: string;
  args?: (string | null)[];
  pipes?: Command[];
  env?: { [key: string]: string | null };
  cwd?: string;
  onOutputLine?: (data: { value: string; type: "stdout" | "stderr" }) => Promise<void>;
};

export type TerminalTextColor = "green" | "red" | "blue" | "yellow" | "magenta" | "cyan" | "white" | "gray";

export type TerminalWriteOptions = {
  color?: TerminalTextColor;
  newLine?: boolean;
};

export interface TaskTerminal {
  execute(options: CommandOptions): Promise<void>;
  write(data: string, options?: TerminalWriteOptions): void;
  /**
   * Open a process-group scope. Every process spawned via the provided
   * `ProcessGroup` is killed when the callback returns (resolve or reject).
   */
  runGroup<T>(callback: (group: ProcessGroup) => Promise<T>): Promise<T>;
}

export type ProcessOutputSink = (chunk: string) => void;

export type ProcessSpec = {
  command: string;
  args?: string[];
  env?: { [key: string]: string | null };
  cwd?: string;
  pty?: boolean;
  // Designates this child as the group's foreground process. Terminal input
  // (including Ctrl+C) is routed to its pty so the kernel delivers SIGINT to
  // its pgroup. Requires `pty: true`. At most one main per group.
  main?: boolean;
};

export type ProcessExit = {
  code: number;
  signal: NodeJS.Signals | null;
};

/**
 * Attach `onData`/`onError` listeners synchronously after `spawn()` — chunks
 * arriving before the first listener attaches are dropped. Under `pty: true`
 * stdout and stderr are merged onto `onData`; `onError` is a no-op.
 */
export type ProcessHandle = {
  readonly pid: number | undefined;
  readonly exit: Promise<ProcessExit>;
  kill(signal?: NodeJS.Signals): void;
  onData(listener: ProcessOutputSink): void;
  onError(listener: ProcessOutputSink): void;
};

export type ProcessGroup = {
  readonly terminal: TaskTerminal;
  spawn(spec: ProcessSpec): ProcessHandle;
};

export const TERMINAL_COLOR_MAP: Record<TerminalTextColor, string> = {
  green: "32",
  red: "31",
  blue: "34",
  yellow: "33",
  magenta: "35",
  cyan: "36",
  white: "37",
  // 90 = "bright black" — renders as dim gray on every terminal theme.
  gray: "90",
};

export class ExecuteTaskError extends Error {
  public command: string;
  public errorCode: number | null;

  constructor(message: string, details: { command: string; errorCode: number | null }) {
    super(message);
    this.command = details.command;
    this.errorCode = details.errorCode;
  }
}

export function cleanCommandArgs(args: (string | null)[] | undefined | null): string[] {
  if (!args) {
    return [];
  }
  return args.filter((arg) => arg !== null);
}
