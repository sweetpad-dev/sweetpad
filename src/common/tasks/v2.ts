import { type ChildProcess, spawn } from "node:child_process";
import { quote } from "shell-quote";
import * as vscode from "vscode";
import { getWorkspacePath } from "../../build/utils";
import type { ExtensionContext } from "../commands";
import { TaskError } from "../errors";
import { prepareEnvVars } from "../helpers";
import { LineBuffer } from "./line-buffer";
import { setTaskPresentationOptions } from "./presentation";
import {
  type CommandOptions,
  ExecuteTaskError,
  type ProcessExit,
  type ProcessGroup,
  type ProcessHandle,
  type ProcessOutputSink,
  type ProcessSpec,
  TERMINAL_COLOR_MAP,
  type TaskTerminal,
  type TerminalWriteOptions,
  cleanCommandArgs,
} from "./types";

type V2GroupChild = {
  readonly pid: number | undefined;
  readonly exit: Promise<ProcessExit>;
  readonly alive: boolean;
  signal: (sig: NodeJS.Signals) => void;
  onData: (listener: ProcessOutputSink) => void;
  onError: (listener: ProcessOutputSink) => void;
};

const V2_GROUP_TERMINATE_TIMEOUT_MS = 2000;

export class TaskTerminalV2 implements vscode.Pseudoterminal, TaskTerminal {
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number>();
  private processes: Set<ChildProcess> = new Set();

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
    if (this.processes.size === 0) {
      return;
    }

    // Collect all PIDs before iterating, since the set may be modified during termination
    const pids: number[] = [];
    for (const proc of this.processes) {
      if (proc.pid) {
        pids.push(proc.pid);
      }
    }

    if (pids.length === 0) {
      return;
    }

    const _kill = (pid: number, signal: string): void => {
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

    // First try to terminate all processes gracefully
    for (const pid of pids) {
      _kill(pid, "SIGTERM");
    }

    // After 5 seconds, we will try to kill remaining processes with SIGKILL with backoff strategy
    const maxAttempts = 3;
    let attempt = 0;
    let timeout = 5000; // 5 seconds

    const _sigkill = () => {
      // Filter to only processes that are still running
      const stillRunning = pids.filter((pid) => {
        for (const proc of this.processes) {
          if (proc.pid === pid && proc.exitCode === null) {
            return true;
          }
        }
        return false;
      });

      if (stillRunning.length === 0) {
        return; // all processes are terminated
      }

      for (const pid of stillRunning) {
        _kill(pid, "SIGKILL");
      }
      attempt++;
      if (attempt < maxAttempts) {
        timeout = timeout * 2; // 10 seconds, 20 seconds, etc.
        setTimeout(_sigkill, timeout);
      }
    };

    setTimeout(_sigkill, timeout);
  }

  // No node-pty in v2: `pty: true` is silently downgraded to plain pipes (no isatty, no TUI fidelity).
  async runGroup<T>(callback: (group: ProcessGroup) => Promise<T>): Promise<T> {
    const children: V2GroupChild[] = [];
    const group: ProcessGroup = {
      terminal: this,
      spawn: (spec) => this.spawnInGroup(spec, children),
    };
    try {
      return await callback(group);
    } finally {
      await this.cleanupGroupChildren(children);
    }
  }

  private spawnInGroup(spec: ProcessSpec, children: V2GroupChild[]): ProcessHandle {
    const env = { ...process.env, ...prepareEnvVars(spec.env ?? {}) };
    const cwd = spec.cwd ?? getWorkspacePath();
    const args = spec.args ?? [];
    const proc = spawn(spec.command, args, {
      cwd,
      env: env as { [key: string]: string },
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    });
    // Tracked in this.processes too so terminal close() kills the whole tree.
    this.processes.add(proc);

    const stdoutListeners: ProcessOutputSink[] = [];
    const stderrListeners: ProcessOutputSink[] = [];
    proc.stdout?.on("data", (data: string | Buffer) => {
      const chunk = data.toString();
      for (const l of stdoutListeners) l(chunk);
    });
    proc.stderr?.on("data", (data: string | Buffer) => {
      const chunk = data.toString();
      for (const l of stderrListeners) l(chunk);
    });

    let alive = true;
    const exit = new Promise<ProcessExit>((resolve) => {
      let resolved = false;
      const finish = (result: ProcessExit) => {
        if (resolved) return;
        resolved = true;
        alive = false;
        this.processes.delete(proc);
        resolve(result);
      };
      proc.on("close", (code, signal) => finish({ code: code ?? -1, signal }));
      proc.on("error", () => finish({ code: -1, signal: null }));
    });

    const child: V2GroupChild = {
      pid: proc.pid,
      exit,
      get alive() {
        return alive;
      },
      signal: (sig) => {
        if (!alive || !proc.pid) return;
        try {
          process.kill(-proc.pid, sig);
        } catch (err) {
          if ((err as NodeJS.ErrnoException).code !== "ESRCH") throw err;
        }
      },
      onData: (l) => {
        stdoutListeners.push(l);
      },
      onError: (l) => {
        stderrListeners.push(l);
      },
    };
    children.push(child);

    return {
      get pid() {
        return child.pid;
      },
      get exit() {
        return child.exit;
      },
      kill: (signal) => child.signal(signal ?? "SIGTERM"),
      onData: child.onData,
      onError: child.onError,
    };
  }

  private async cleanupGroupChildren(children: V2GroupChild[]): Promise<void> {
    const alive = children.filter((c) => c.alive);
    if (alive.length === 0) return;

    for (const c of alive) {
      try {
        c.signal("SIGTERM");
      } catch {}
    }

    await Promise.race([
      Promise.all(alive.map((c) => c.exit.catch(() => undefined))),
      new Promise<void>((resolve) => setTimeout(resolve, V2_GROUP_TERMINATE_TIMEOUT_MS)),
    ]);

    for (const c of children) {
      if (c.alive) {
        try {
          c.signal("SIGKILL");
        } catch {}
      }
    }
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
      const workspacePath = options.cwd ?? getWorkspacePath();

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
      const childProcess = spawn(command, {
        // run command in shell to support pipes
        shell: true,
        // in order to be able to kill the whole process group
        // run it in a separate process group
        detached: true,
        env: env,
        cwd: workspacePath,
      });
      this.processes.add(childProcess);
      childProcess.stderr?.on("data", (data: string | Buffer): void => {
        const output = data.toString();
        this.write(output, { color: "yellow" });
        hasOutput = true;

        stderrBuffer.append(output);
      });
      childProcess.stdout?.on("data", (data: string | Buffer): void => {
        const output = data.toString();
        this.write(output);
        hasOutput = true;

        stdouBuffer.append(output);
      });
      childProcess.stdin?.on("data", (data: string | Buffer): void => {
        const input = data.toString();
        this.write(input);
        hasOutput = true;
      });
      childProcess.on("close", (code) => {
        // make space between command output and next command or error message
        // when we don't have any output, we already have a new line after command
        if (hasOutput) {
          this.writeLine();
        }

        stdouBuffer.flush();
        stderrBuffer.flush();

        this.processes.delete(childProcess);
        if (code !== 0) {
          reject(
            new ExecuteTaskError("Command returned non-zero exit code", { command: commandPrint, errorCode: code }),
          );
        } else {
          resolve();
        }
      });
      childProcess.on("error", (error) => {
        if (hasOutput) {
          this.writeLine();
        }

        this.processes.delete(childProcess);
        reject(new ExecuteTaskError("Error running command", { command: commandPrint, errorCode: null }));
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

/**
 * V2 version of the task runner that uses the `vscode.CustomExecution` API and each execution
 * just adds a new command to the same terminal, it allows to have a single terminal for all
 * commands and cancel them all at once.
 */
export async function runTaskV2<TMetadata>(
  context: ExtensionContext,
  options: {
    name: string;
    source?: string;
    error?: string;
    callback: (terminal: TaskTerminal) => Promise<void>;
    problemMatchers?: string[];
    lock: string;
    metadata?: TMetadata;
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
      metadata: options.metadata,
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
