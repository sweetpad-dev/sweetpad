import * as vscode from "vscode";
import { TaskError } from "./errors";
import { ChildProcess, spawn } from "child_process";
import { quote } from "shell-quote";
import { getWorkspaceConfig } from "./config";
import { ExtensionContext } from "./commands";
import path from "path";

type ExecutorVersion = "v1" | "v2";

type Command = {
  command: string;
  args?: string[];
  setvbuf?: boolean;
};

type CommandOptions = {
  command: string;
  args?: string[];
  pipes?: Command[];
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
}

export function getExecutorVersion(): ExecutorVersion {
  return getWorkspaceConfig<ExecutorVersion>("system.executor") ?? "v2";
}

export class TaskTerminalV2 implements vscode.Pseudoterminal, TaskTerminal {
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number>();
  private process: ChildProcess | null = null;

  constructor(
    private context: ExtensionContext,
    private options: {
      callback: (terminal: TaskTerminalV2) => Promise<void>;
    }
  ) {}

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  private command(command: string, args?: string[]): string {
    return quote([command, ...(args ?? [])]);
  }

  private createCommandLine(options: CommandOptions): string {
    const mainCommand = quote([options.command, ...(options.args ?? [])]);

    if (!options.pipes) {
      return mainCommand;
    }

    // Combine them into a big pipe with error propagation
    const commands = [mainCommand];
    commands.push(
      ...options.pipes.map((pipe) => {
        const command = this.command(pipe.command, pipe.args);
        if (pipe.setvbuf) {
          const env = {
            DYLD_INSERT_LIBRARIES: path.join(this.context.extensionPath, "out/setvbuf_universal.so"),
            DYLD_FORCE_FLAT_NAMESPACE: "y",
          };

          // Create a prefix with environment variables, like:
          // ENV_1=VALUE_1 ENV_2=VALUE_2
          const envPrefix = Object.entries(env)
            .map(([key, value]) => `${key}=${quote([value])}`)
            .join(" ");

          // Append the prefix before the command
          return `${envPrefix} ${command}`;
        }
        return command;
      })
    );
    return `set -o pipefail;  ${commands.join(" | ")}`;
  }

  private write(data: string, options?: TerminalWriteOptions): void {
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
      // TODO: handle it better, bacause pid can be assigned
      // to another process and we can kill it by mistake
      const pid = this.process?.pid;
      if (pid) {
        console.log("Killing process");
        process.kill(-pid, "SIGINT");
      }
    } else {
      this.write(data);
    }
  }

  /**
   * This method you can call in your callback to execute a command in the terminal.
   */
  async execute(options: CommandOptions): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const command = this.createCommandLine(options);
      const commandPrint = this.command(options.command, options.args);

      this.writeLine(`🚀 Executing command:`);
      this.writeLine(commandPrint, { color: "green" });
      this.writeLine();

      this.process = spawn(command, {
        shell: true,
        detached: true,
        env: {
          ...process.env,
          FORCE_COLOR: "1",
        },
      });
      this.process.stderr?.on("data", (data: string | Buffer): void => {
        const output = data.toString();
        this.write(output, { color: "red" });
      });
      this.process.stdout?.on("data", (data: string | Buffer): void => {
        const output = data.toString();
        this.write(output);
      });
      this.process.stdin?.on("data", (data: string | Buffer): void => {
        const input = data.toString();
        this.write(input);
      });
      this.process.on("close", (code) => {
        this.process = null;
        if (code !== 0) {
          reject(new ExecuteTaskError("Command failed", { command, errorCode: code }));
        } else {
          resolve();
        }
      });
      this.process.on("error", (error) => {
        reject(new ExecuteTaskError("Command failed", { command, errorCode: null }));
        this.process = null;
      });
    });
  }

  open(initialDimensions: vscode.TerminalDimensions | undefined): void {
    void this.start();
  }

  close(): void {
    this.closeSuccessfully();
  }

  private closeSuccessfully(): void {
    this.closeTerminal(0, "✅ Task completed", { color: "green" });
  }

  private closeTerminal(code: number, message: string, options?: TerminalWriteOptions): void {
    this.writeLine();
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
      let options: TerminalWriteOptions = { color: "red" };

      // Handling specific error types
      if (error instanceof ExecuteTaskError) {
        errorCode = error.errorCode ?? errorCode;
        errorMessage = `🚫 ${error.message}`;

        // If task was canceled, change message color to green
        if (errorCode === 130) {
          options.color = "green";
          this.closeTerminal(0, `🫡 Task was canceled by user`, options);
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
    }
  ) {}

  private command(command: string, args?: string[]): string {
    return quote([command, ...(args ?? [])]);
  }

  private commandLine(options: CommandOptions): string {
    const mainCommand = quote([options.command, ...(options.args ?? [])]);

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
      new vscode.ShellExecution(command)
    );

    const execution = await vscode.tasks.executeTask(task);

    return new Promise((resolve, reject) => {
      const disposable = vscode.tasks.onDidEndTaskProcess((e) => {
        if (e.execution === execution) {
          disposable.dispose();
          if (e.exitCode !== 0) {
            const message = this.options.error ?? `Error running task '${this.options.name}'`;
            const error = new TaskError(message, {
              name: this.options.name,
              soruce: this.options.source,
              command: options.command,
              args: options.args ?? [],
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

/**
 * V1 version of the task runner that uses the `vscode.Task` API for each execution.
 */
export async function runTaskV1(
  context: ExtensionContext,
  options: {
    name: string;
    source?: string;
    error?: string;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }
): Promise<void> {
  const terminal = new TaskTerminalV1(context, options);
  await options.callback(terminal);
}

/**
 * V2 version of the task runner that uses the `vscode.CustomExecution` API and each execution
 * just adds a new command to the same terminal, it allows to have a single terminal for all
 * commands and cancel them all at once.
 */
export async function runTaskV2(
  context: ExtensionContext,
  options: {
    name: string;
    source?: string;
    error?: string;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }
): Promise<void> {
  const task = new vscode.Task(
    { type: "custom" },
    vscode.TaskScope.Workspace,
    options.name,
    options.source ?? "sweetpad",
    new vscode.CustomExecution(async () => {
      return new TaskTerminalV2(context, {
        callback: options.callback,
      });
    })
  );

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
    callback: (terminal: TaskTerminal) => Promise<void>;
  }
): Promise<void> {
  const executorVersion = getExecutorVersion();
  switch (executorVersion) {
    case "v1":
      return await runTaskV1(context, options);
    case "v2":
      return await runTaskV2(context, options);
    default:
      throw new Error(`Unknown executor: ${executorVersion}`);
  }
}
