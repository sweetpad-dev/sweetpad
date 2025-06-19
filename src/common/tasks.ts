import { type ChildProcess, spawn } from "node:child_process";
import { quote } from "shell-quote";
import * as vscode from "vscode";
import { getWorkspacePath } from "../build/utils";
import type { ExtensionContext } from "./commands";
import { getWorkspaceConfig } from "./config";
import { TaskError } from "./errors";
import { prepareEnvVars } from "./helpers";

type TaskExecutor = "v1" | "v2";

export type Command = {
  command: string;
  args?: string[];
};

export type CommandOptions = {
  command: string;
  args?: (string | null)[];
  pipes?: Command[];
  env?: { [key: string]: string | null };
  onOutputLine?: (data: { value: string; type: "stdout" | "stderr" }) => Promise<void>;
};

type TerminalTextColor = "green" | "red" | "blue" | "yellow" | "magenta" | "cyan" | "white";
type TerminalWriteOptions = {
  color?: TerminalTextColor;
  newLine?: boolean;
};

class ExecuteTaskError extends Error {
  public command: string;
  public errorCode: number | null;

  constructor(message: string, details: { command: string; errorCode: number | null }) {
    super(message);
    this.command = details.command;
    this.errorCode = details.errorCode;
  }
}

const TERMINAL_COLOR_MAP: Record<TerminalTextColor, string> = {
  green: "32",
  red: "31",
  blue: "34",
  yellow: "33",
  magenta: "35",
  cyan: "36",
  white: "37",
};

/**
 * Interface that will be passed as argument to the
 * callback function in the `runTask` function.
 */
export interface TaskTerminal {
  execute(options: CommandOptions): Promise<void>;
  write(data: string, options?: TerminalWriteOptions): void;
}

export function getTaskExecutorName(): TaskExecutor {
  return getWorkspaceConfig("system.taskExecutor") ?? "v2";
}

/**
 * Remove all null values from the array of command arguments
 */
function cleanCommandArgs(args: (string | null)[] | undefined | null): string[] {
  if (!args) {
    return [];
  }
  return args.filter((arg) => arg !== null);
}

export function setTaskPresentationOptions(task: vscode.Task): void {
  const autoRevealTerminal = getWorkspaceConfig("system.autoRevealTerminal") ?? true;
  task.presentationOptions = {
    // terminal will be revealed, if auto reveal is enabled
    reveal: autoRevealTerminal ? vscode.TaskRevealKind.Always : vscode.TaskRevealKind.Never,
  };
}

/**
 * Collect stdout or stderr output and send it line by line to the callback
 */
class LineBuffer {
  public buffer = "";
  public enabled = true;
  public callback: (line: string) => void;

  constructor(options: { enabled: boolean; callback: (line: string) => void }) {
    this.enabled = options.enabled;
    this.callback = options.callback;
  }

  append(data: string): void {
    if (!this.enabled) return;

    this.buffer += data;

    const lines = this.buffer.split("\n");

    // last line can be not finished yet, so we need to keep it and send to callback later
    this.buffer = lines.pop() ?? "";

    // send all lines in buffer to callback, except last one
    for (const line of lines) {
      this.callback(line);
    }
  }

  flush(): void {
    if (!this.enabled) return;

    if (this.buffer) {
      this.callback(this.buffer);
      this.buffer = "";
    }
  }
}

export class TaskTerminalV2 implements vscode.Pseudoterminal, TaskTerminal {
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number>();
  private process: ChildProcess | null = null;

  constructor(
    private context: ExtensionContext,
    private options: {
      callback: (terminal: TaskTerminalV2) => Promise<void>;
    },
  ) {}

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  private command(command: string, args?: string[]): string {
    return quote([command, ...(args ?? [])]);
  }

  private async createCommandLine(options: CommandOptions): Promise<string> {
    const args = cleanCommandArgs(options.args);
    const mainCommand = quote([options.command, ...args]);

    if (!options.pipes) {
      return mainCommand;
    }

    // Combine them into a big pipe with error propagation
    const commands = [mainCommand];
    commands.push(...options.pipes.map((pipe) => this.command(pipe.command, pipe.args)));
    return `set -o pipefail;  ${commands.join(" | ")}`;
  }

  write(data: string, options?: TerminalWriteOptions): void {
    const color = options?.color;

    // Replace all \r\n or \n with \r\n CR+LF (VSCode requires this format for new lines)
    let output = data.replace(/\r?\n/g, "\r\n");
    if (color) {
      const colorCode = TERMINAL_COLOR_MAP[color];
      output = `\x1b[${colorCode}m${output}\x1b[0m`;
    }
    if (options?.newLine) {
      output += "\r\n";
    }
    this.writeEmitter.fire(output);
  }

  private writeLine(line?: string, options?: TerminalWriteOptions): void {
    this.write(line ?? "", {
      ...options,
      newLine: true,
    });
  }

  handleInput(data: string): void {
    if (data === "\x03") {
      // Handle Ctrl+C
      this.writeLine("^C");
      this.terminateProcess();
    } else {
      this.write(data);
    }
  }

  private terminateProcess(): void {
    const pid = this.process?.pid;
    if (!pid) {
      return;
    }

    const _kill = (signal: string): void => {
      try {
        process.kill(-pid, signal);
      } catch (e) {
        // process does not exist, then it's already terminated
        if ((e as NodeJS.ErrnoException).code === "ESRCH") {
          return;
        }
        throw e;
      }
    };

    // First try to terminate the process gracefully
    _kill("SIGTERM");

    // After 5 seconds, we will try to kill it with SIGKILL with backoff strategy
    const maxAttempts = 3;
    let attempt = 0;
    let timeout = 5000; // 5 seconds

    const _sigkill = () => {
      if (!this.process || this.process.exitCode !== null) {
        return; // the process is already terminated
      }

      _kill("SIGKILL");
      attempt++;
      if (attempt < maxAttempts) {
        timeout = timeout * 2; // 10 seconds, 20 seconds, etc.
        setTimeout(_sigkill, timeout);
      }
    };

    setTimeout(_sigkill, timeout);
  }

  /**
   * This method you can call in your callback to execute a command in the terminal.
   */
  async execute(options: CommandOptions): Promise<void> {
    const command = await this.createCommandLine(options);

    const args = cleanCommandArgs(options.args);
    const commandPrint = this.command(options.command, args);

    this.writeLine("🚀 Executing command:");
    this.writeLine(commandPrint, { color: "green" });
    this.writeLine();

    let hasOutput = false;

    return new Promise<void>((resolve, reject) => {
      const workspacePath = getWorkspacePath();

      // Collect lines and send them to the callback
      // This is usefull when you need to listen to task output and make some actions based on it
      const stdouBuffer = new LineBuffer({
        enabled: !!options.onOutputLine,
        callback: (line) => {
          options.onOutputLine?.({ value: line, type: "stdout" });
        },
      });
      const stderrBuffer = new LineBuffer({
        enabled: !!options.onOutputLine,
        callback: (line) => {
          options.onOutputLine?.({ value: line, type: "stderr" });
        },
      });

      const env = { ...process.env, ...prepareEnvVars(options.env) };
      this.process = spawn(command, {
        // run command in shell to support pipes
        shell: true,
        // in order to be able to kill the whole process group
        // run it in a separate process group
        detached: true,
        env: env,
        cwd: workspacePath,
      });
      this.process.stderr?.on("data", (data: string | Buffer): void => {
        const output = data.toString();
        this.write(output, { color: "yellow" });
        hasOutput = true;

        stderrBuffer.append(output);
      });
      this.process.stdout?.on("data", (data: string | Buffer): void => {
        const output = data.toString();
        this.write(output);
        hasOutput = true;

        stdouBuffer.append(output);
      });
      this.process.stdin?.on("data", (data: string | Buffer): void => {
        const input = data.toString();
        this.write(input);
        hasOutput = true;
      });
      this.process.on("close", (code) => {
        // make space between command output and next command or error message
        // when we don't have any output, we already have a new line after command
        if (hasOutput) {
          this.writeLine();
        }

        stdouBuffer.flush();
        stderrBuffer.flush();

        this.process = null;
        if (code !== 0) {
          reject(
            new ExecuteTaskError("Command returned non-zero exit code", { command: commandPrint, errorCode: code }),
          );
        } else {
          resolve();
        }
      });
      this.process.on("error", (error) => {
        if (hasOutput) {
          this.writeLine();
        }

        reject(new ExecuteTaskError("Error running command", { command: commandPrint, errorCode: null }));
        this.process = null;
      });
    });
  }

  open(initialDimensions: vscode.TerminalDimensions | undefined): void {
    void this.start();
  }

  close(): void {
    this.terminateProcess();
    this.closeSuccessfully();
  }

  private closeSuccessfully(): void {
    this.closeTerminal(0, "✅ Task completed", { color: "green" });
  }

  private closeTerminal(code: number, message: string, options?: TerminalWriteOptions): void {
    this.writeLine(message, options);
    this.writeLine();
    this.closeEmitter.fire(code);
  }

  private async start(): Promise<void> {
    try {
      await this.options.callback(this);
    } catch (error) {
      // Default values for error handling
      let errorCode = -1;
      let errorMessage = `🚷 ${error?.toString()}`;
      const options: TerminalWriteOptions = { color: "red" };

      // Handling specific error types
      if (error instanceof ExecuteTaskError) {
        errorCode = error.errorCode ?? errorCode;
        errorMessage = `🚫 ${error.message}`;

        // If task was canceled, change message color to green
        if (errorCode === 130) {
          options.color = "yellow";
          this.closeTerminal(0, "🫡 Command was cancelled by user", options);
          return;
        }
      }

      // Closing the terminal with error information
      this.closeTerminal(errorCode, errorMessage, options);
      return;
    }
    this.closeSuccessfully();
  }
}

export class TaskTerminalV1 implements TaskTerminal {
  constructor(
    private context: ExtensionContext,
    private options: {
      name: string;
      source?: string;
      error?: string;
      problemMatchers?: string[];
    },
  ) {}

  write(data: string, options?: TerminalWriteOptions): void {
    this.execute({
      command: "echo",
      args: [data],
    });
  }

  private command(command: string, args?: string[]): string {
    return quote([command, ...(args ?? [])]);
  }

  private commandLine(options: CommandOptions): string {
    const args = cleanCommandArgs(options.args);
    const mainCommand = quote([options.command, ...args]);

    if (!options.pipes) {
      return mainCommand;
    }

    // Combine them into a big pipe with error propagation
    const commands = [mainCommand];
    commands.push(...options.pipes.map((pipe) => this.command(pipe.command, pipe.args)));
    return `set -o pipefail;  ${commands.join(" | ")}`;
  }

  async execute(options: CommandOptions): Promise<void> {
    const command = this.commandLine(options);

    const task = new vscode.Task(
      { type: "shell" },
      vscode.TaskScope.Workspace,
      this.options.name,
      this.options.source ?? "sweetpad",
      new vscode.ShellExecution(command),
      this.options.problemMatchers,
    );
    setTaskPresentationOptions(task);

    const execution = await vscode.tasks.executeTask(task);

    return new Promise((resolve, reject) => {
      const disposable = vscode.tasks.onDidEndTaskProcess((e) => {
        if (e.execution === execution) {
          disposable.dispose();
          if (e.exitCode !== 0) {
            const message = this.options.error ?? `Error running task '${this.options.name}'`;
            const args = cleanCommandArgs(options.args);
            const error = new TaskError(message, {
              name: this.options.name,
              soruce: this.options.source,
              command: options.command,
              args: args,
              errorCode: e.exitCode,
            });
            reject(error);
          } else {
            resolve();
          }
        }
      });
    });
  }
}

export class TaskTerminalV1Parent implements vscode.Pseudoterminal {
  public writeEmitter = new vscode.EventEmitter<string>();
  public closeEmitter = new vscode.EventEmitter<number>();

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  writePlaceholderText(): void {
    this.writeEmitter.fire("====> It's parent task, just ignore it\r\n");
  }

  open(): void {
    this.writePlaceholderText();
    this.closeEmitter.fire(0);
  }

  close(): void {
    this.writeEmitter.dispose();
    this.closeEmitter.fire(0);
    this.closeEmitter.dispose();
  }
}

/**
 * V1 version of the task runner that uses the `vscode.Task` API for each execution.
 */
async function runTaskV1(
  context: ExtensionContext,
  options: {
    name: string;
    source?: string;
    error?: string;
    callback: (terminal: TaskTerminal) => Promise<void>;
  },
): Promise<void> {
  const terminal = new TaskTerminalV1(context, options);
  await options.callback(terminal);
}

/**
 * V2 version of the task runner that uses the `vscode.CustomExecution` API and each execution
 * just adds a new command to the same terminal, it allows to have a single terminal for all
 * commands and cancel them all at once.
 */
async function runTaskV2(
  context: ExtensionContext,
  options: {
    name: string;
    source?: string;
    error?: string;
    callback: (terminal: TaskTerminal) => Promise<void>;
    problemMatchers?: string[];
    lock: string;
    terminateLocked: boolean;
  },
): Promise<void> {
  // Termiate all previous tasks with the same lockId
  if (options.terminateLocked) {
    const tasks = vscode.tasks.taskExecutions.filter((task) => task.task.definition.lockId === options.lock);
    for (const task of tasks) {
      task.terminate();
    }
  }

  const currentScope = context.getExecutionScope();
  const task = new vscode.Task(
    {
      type: "custom",
      lockId: options.lock,
    },
    vscode.TaskScope.Workspace,
    options.name,
    options.source ?? "sweetpad",
    new vscode.CustomExecution(async () => {
      return new TaskTerminalV2(context, {
        callback: (terminal) => {
          // we propagate current command to the callback because vscode.CustomExecution
          // breaks the context that we use to show progress
          return context.setExecutionScope(currentScope, () => {
            return options.callback(terminal);
          });
        },
      });
    }),
    options.problemMatchers,
  );
  setTaskPresentationOptions(task);

  const execution = await vscode.tasks.executeTask(task);

  return new Promise((resolve, reject) => {
    const disposable = vscode.tasks.onDidEndTaskProcess((e) => {
      if (e.execution === execution) {
        disposable.dispose();
        if (e.exitCode !== 0) {
          const message = options.error ?? `Error running task '${options.name}'`;
          const error = new TaskError(message, {
            name: options.name,
            soruce: options.source,
            errorCode: e.exitCode,
          });
          reject(error);
        } else {
          resolve();
        }
      }
    });
  });
}

/**
 * Run a tasks in the terminal
 */
export async function runTask(
  context: ExtensionContext,
  options: {
    name: string;
    source?: string;
    error?: string;
    problemMatchers?: string[];
    lock: string;
    terminateLocked: boolean;
    callback: (terminal: TaskTerminal) => Promise<void>;
  },
): Promise<void> {
  const name = getTaskExecutorName();
  switch (name) {
    case "v1":
      return await runTaskV1(context, options);
    case "v2":
      return await runTaskV2(context, options);
    default:
      throw new Error(`Unknown executor: ${name}`);
  }
}
