import * as vscode from "vscode";
import { TaskError } from "./errors";
import { ChildProcess, spawn } from "child_process";
import { quote } from "shell-quote";
import { execa } from "execa";

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

class TaskTerminal implements vscode.Pseudoterminal {
  private abortController: AbortController | null = null;
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number>();
  private process: ChildProcess | null = null;

  constructor(
    private options: {
      callback: (terminal: TaskTerminal) => Promise<void>;
    }
  ) {}

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  command(command: string, args?: string[]): string {
    return quote([command, ...(args ?? [])]);
  }

  createCommandLine(options: CommandOptions): string {
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
          //todo: use the correct path
          return `DYLD_INSERT_LIBRARIES=/Users/hyzyla/Developer/sweetpad/out/setvbuf_universal.so DYLD_FORCE_FLAT_NAMESPACE=y ${command}`;
        }
        return command;
      })
    );
    return `set -o pipefail;  ${commands.join(" | ")}`;
  }

  write(data: string, options?: TerminalWriteOptions): void {
    const color = options?.color;
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

  writeLine(line?: string, options?: TerminalWriteOptions): void {
    this.write(line ?? "", {
      ...options,
      newLine: true,
    });
  }

  handleInput(data: string): void {
    console.log(`HANDLE INPUT ${data} stringified: ${data.toString()}`);

    if (data === "\x03") {
      this.writeLine("^C");
    } else {
      this.write(data);
    }

    // Handle Ctrl+C
    if (data === "\x03") {
      this.abortController?.abort();

      return;
    }
  }

  async execute(options: CommandOptions): Promise<void> {
    return this.execute_nonpty(options);
  }

  async execute_nonpty(options: CommandOptions): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const command = this.createCommandLine(options);
      const commandPrint = this.command(options.command, options.args);

      this.writeLine(`🚀 Executing command:`);
      this.writeLine(commandPrint, { color: "green" });
      this.writeLine();

      const execaAny = execa as any;
      this.abortController = new AbortController();

      this.process = execaAny(command, {
        cwd: vscode.workspace.rootPath, // todo: use workspace folder
        shell: true,
        env: {
          ...process.env,
          TERM: "xterm-256color",
        },
        cancelSignal: this.abortController.signal,
        buffer: false,
      }) as ChildProcess;
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

      this.process.on("close", (code, signal) => {
        if (code !== 0) {
          reject(new ExecuteTaskError("Command failed", { command, errorCode: code }));
        } else {
          resolve();
        }
        this.closeTerminal(0, "🧨 Task was terminated");
        this.process = null;
      });
      this.process.on("error", (error) => {
        reject(new ExecuteTaskError("Command failed", { command, errorCode: null }));
        this.closeTerminal(0, "🧨 Task was terminated");
        this.process = null;
      });
    });
  }

  open(initialDimensions: vscode.TerminalDimensions | undefined): void {
    void this.start();
  }

  close(): void {
    this.closeTerminal(0, "✅ Task completed", { color: "green" });
  }

  closeTerminal(code: number, message: string, options?: TerminalWriteOptions): void {
    this.writeLine();
    this.writeLine(message, options);
    this.closeEmitter.fire(code);
  }

  async start(): Promise<void> {
    try {
      await this.options.callback(this);
    } catch (error) {
      const options: TerminalWriteOptions = { color: "red" };

      let code: number, message: string;
      if (error instanceof ExecuteTaskError) {
        code = error.errorCode ?? -1;
        message = `🚫 ${error.message}`;
      } else {
        code = -1;
        message = `🚷 ${error?.toString()}`;
      }

      this.closeTerminal(code, message, options);
      return;
    }
    this.close();
  }
}

export async function runCustomTaskV2(options: {
  name: string;
  source?: string;
  error?: string;
  callback: (terminal: TaskTerminal) => Promise<void>;
}): Promise<void> {
  const task = new vscode.Task(
    { type: "custom" },
    vscode.TaskScope.Workspace,
    options.name,
    options.source ?? "sweetpad",
    new vscode.CustomExecution(async () => {
      return new TaskTerminal({
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
            command: "TODO",
            args: ["TODO"],
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
