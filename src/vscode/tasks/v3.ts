import { type ChildProcess, spawn as childSpawn } from "node:child_process";

import type { IPty } from "node-pty";
import { quote } from "shell-quote";
import * as vscode from "vscode";

import type { ConfigProvider } from "../../core/config/types";
import { TaskError } from "../../core/errors";
import { prepareEnvVars } from "../../core/helpers";
import type { Logger } from "../../core/logger/types";
import { LineBuffer } from "../../core/tasks/line-buffer";
import { getShellEnv } from "../../core/tasks/shell-env";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { setTaskPresentationOptions } from "./presentation";
import { loadNodePty } from "./pty";
import type { VsCodeTaskRunnerDeps } from "./run";

function shellEnvOptions(config: ConfigProvider, logger: Logger) {
  return {
    logger,
    shell: config.get("shellEnv.shell"),
    timeoutMs: config.get("shellEnv.timeout"),
  };
}
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
} from "../../core/tasks/types";
import { runTaskV2 } from "./v2";

// CSI-only strip. OSC/APC are rare enough to ignore.
const ESC = String.fromCharCode(0x1b);
const ANSI_STRIP_REGEX = new RegExp(`${ESC}\\[[0-9;?]*[a-zA-Z]`, "g");

const TERMINATE_TIMEOUT_MS = 2000;

type GroupChild = {
  readonly pid: number | undefined;
  readonly exit: Promise<ProcessExit>;
  readonly alive: boolean;
  signal: (sig: NodeJS.Signals) => void;
  onData: (listener: ProcessOutputSink) => void;
  onError: (listener: ProcessOutputSink) => void;
};

// node-pty reports the killing signal as a POSIX number; map the ones we use.
function ptySignalName(num: number | undefined): NodeJS.Signals | null {
  switch (num) {
    case 1:
      return "SIGHUP";
    case 2:
      return "SIGINT";
    case 3:
      return "SIGQUIT";
    case 9:
      return "SIGKILL";
    case 15:
      return "SIGTERM";
    default:
      return null;
  }
}

export class TaskTerminalV3 implements vscode.Pseudoterminal, TaskTerminal {
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number>();
  private currentPty: IPty | undefined;
  private mainPty: IPty | undefined;
  private groupPtys = new Set<IPty>();
  private groupPipes = new Set<ChildProcess>();
  private dims: { cols: number; rows: number } = { cols: 80, rows: 30 };
  private closed = false;
  private closeFired = false;
  // Set when the user types Ctrl+C (0x03) while a runGroup is active. Lets us surface
  // cancellation even when main exits cleanly (e.g. simctl handling SIGINT, or a Swift
  // app catching it and returning 0). Reset at every runGroup entry.
  private userInterrupted = false;
  private inGroup = false;

  constructor(
    private options: {
      callback: (terminal: TaskTerminalV3) => Promise<void>;
      workspaceRoot: WorkspaceRoot;
      config: ConfigProvider;
      logger: Logger;
    },
  ) {}

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  open(initialDimensions: vscode.TerminalDimensions | undefined): void {
    if (initialDimensions) {
      this.dims = { cols: initialDimensions.columns, rows: initialDimensions.rows };
    }
    void this.start();
  }

  close(): void {
    this.closed = true;
    this.killCurrentPty();
    this.killGroupChildren();
    // VS Code only treats the task as ended once the close emitter fires. If close()
    // arrives externally (user closed the terminal, vscode.tasks.terminate, etc.)
    // before start() reaches its own closeTerminal call, the surrounding runTaskV3
    // promise would otherwise wait for an onDidEndTaskProcess event that never
    // comes, leaving the build hung.
    if (!this.closeFired) {
      this.closeFired = true;
      this.closeEmitter.fire(-1);
    }
  }

  setDimensions(dimensions: vscode.TerminalDimensions): void {
    this.dims = { cols: dimensions.columns, rows: dimensions.rows };
    const resize = (p: IPty) => {
      try {
        p.resize(dimensions.columns, dimensions.rows);
      } catch (err) {
        this.options.logger.debug("pty.resize failed", {
          error: err instanceof Error ? err.message : String(err),
        });
      }
    };
    if (this.currentPty) resize(this.currentPty);
    for (const p of this.groupPtys) resize(p);
  }

  handleInput(data: string): void {
    // Ctrl+C (0x03) is delivered to the foreground pgroup by the tty — no interception.
    // Outside a runGroup callback this targets the executing pty; inside one it targets
    // whichever child was spawned with `main: true` so SIGINT lands on the app, not the
    // sidecars (cleanupGroup tears those down once the callback returns).
    //
    // Caveat: if a runGroup is active but no child was spawned with `main: true`, `target`
    // is undefined and the keystrokes go nowhere — the user can't interrupt that callback
    // until it returns naturally. Every current consumer goes through MainExecutable, which
    // always sets `main: true`, so this path isn't reached in practice.
    if (this.inGroup && data.includes("\x03")) {
      this.userInterrupted = true;
    }
    const target = this.currentPty ?? this.mainPty;
    target?.write(data);
  }

  write(data: string, options?: TerminalWriteOptions): void {
    const color = options?.color;
    // VS Code requires CRLF line endings in Pseudoterminal output.
    let output = data.replace(/\r?\n/g, "\r\n");
    if (color) {
      const colorCode = TERMINAL_COLOR_MAP[color];
      output = `${ESC}[${colorCode}m${output}${ESC}[0m`;
    }
    if (options?.newLine) {
      output += "\r\n";
    }
    this.writeEmitter.fire(output);
  }

  private writeLine(line?: string, options?: TerminalWriteOptions): void {
    this.write(line ?? "", { ...options, newLine: true });
  }

  async execute(options: CommandOptions): Promise<void> {
    if (this.closed) {
      throw new ExecuteTaskError("Terminal is closed", { command: options.command, errorCode: null });
    }

    const nodePty = loadNodePty();
    if (!nodePty) {
      // Unreachable: runTaskV3 falls back to v2 when node-pty is missing.
      throw new ExecuteTaskError("node-pty is not available", { command: options.command, errorCode: null });
    }

    const args = cleanCommandArgs(options.args);
    const commandPrint = quoteForDisplay(options.command, args);

    this.writeLine("🚀 Executing command:");
    this.writeLine(commandPrint, { color: "green" });
    this.writeLine();

    const cwd = options.cwd ?? this.options.workspaceRoot.getPath();
    const env = await this.buildExecuteEnv(options);

    // Three ways to launch, selected by (pipes?, closeStdin?):
    //
    // 1. pipes + (closeStdin optional)
    //    /bin/bash -c 'set -o pipefail; CMD ARGS [< /dev/null] | xcbeautify'
    //    Bash orchestrates the pipeline and stays alive to apply pipefail
    //    semantics. < /dev/null is injected only on the first stage when
    //    closeStdin is set (B, C, … inherit stdin from the pipe regardless).
    //
    // 2. no pipes, closeStdin = true
    //    /bin/bash -c '"$@" < /dev/null' bash CMD ARGS
    //    Bash runs the single command with fd 0 → /dev/null, waits, exits.
    //    We deliberately do NOT use `exec` here so bash stays alive as the
    //    parent — same process topology as path 1. That keeps killCurrentPty
    //    (which signals pty.pid via node-pty) uniform whether xcbeautify is
    //    piped in or not. The `"$@"` form passes argv verbatim so paths with
    //    spaces/quotes don't need shell-escaping.
    //
    // 3. no pipes, closeStdin = false
    //    Direct execvp of CMD ARGS. No shell.
    //
    // Why closeStdin exists at all (issue #240): xcodebuild, when its own
    // stdin looks like a TTY, opens an interactive pipe for scheme pre-action
    // scripts and never writes to it. Any pre-action doing `read` hangs
    // forever. Closing fd 0 on xcodebuild makes its isatty(0) check fail, so
    // it doesn't set up that pipe in the first place — same path Xcode IDE
    // takes when it invokes xcodebuild.
    //
    // Why a shell wrapper rather than per-fd stdio: node-pty's whole design
    // is one bidirectional pty connected to fd 0, 1, and 2 of the child;
    // there's no API to redirect fd 0 separately. A shell `<` redirect is the
    // only way to split them while keeping the pty on stdout/stderr.
    const closeStdin = options.closeStdin ?? false;
    let spawnCmd: string;
    let spawnArgs: string[];
    if (options.pipes && options.pipes.length > 0) {
      spawnCmd = "/bin/bash";
      spawnArgs = ["-c", buildPipelineScript(options.command, args, options.pipes, closeStdin)];
    } else if (closeStdin) {
      spawnCmd = "/bin/bash";
      spawnArgs = ["-c", '"$@" < /dev/null', "bash", options.command, ...args];
    } else {
      spawnCmd = options.command;
      spawnArgs = args;
    }

    this.options.logger.debug("v3 spawn", {
      command: spawnCmd,
      args: spawnArgs,
      cwd,
      closeStdin,
      hasPipes: !!options.pipes && options.pipes.length > 0,
      original: { command: options.command, args },
    });

    const pty = nodePty.spawn(spawnCmd, spawnArgs, {
      name: "xterm-256color",
      cwd,
      env: env as { [key: string]: string },
      cols: this.dims.cols,
      rows: this.dims.rows,
    });
    this.currentPty = pty;

    const observerBuffer = options.onOutputLine
      ? new LineBuffer({
          enabled: true,
          callback: (line) => {
            options.onOutputLine?.({ value: line, type: "stdout" });
          },
        })
      : undefined;

    pty.onData((chunk) => {
      // Raw chunks preserve TUI redraws (\r progress bars etc.). Observer gets
      // ANSI-stripped lines for callers that scrape output (e.g. test runner).
      this.writeEmitter.fire(chunk);
      if (observerBuffer) observerBuffer.append(stripAnsi(chunk));
    });

    const exitCode = await new Promise<number>((resolve) => {
      pty.onExit(({ exitCode: code, signal }) => {
        // SIGINT → 130 so the error-handling path recognizes user cancellation.
        if (signal === 2) {
          resolve(130);
          return;
        }
        resolve(code ?? -1);
      });
    });

    observerBuffer?.flush();
    if (this.currentPty === pty) {
      this.currentPty = undefined;
    }

    if (exitCode !== 0) {
      throw new ExecuteTaskError("Command returned non-zero exit code", {
        command: commandPrint,
        errorCode: exitCode,
      });
    }
  }

  async runGroup<T>(callback: (group: ProcessGroup) => Promise<T>): Promise<T> {
    if (this.closed) {
      throw new ExecuteTaskError("Terminal is closed", { command: "<group>", errorCode: null });
    }
    const nodePty = loadNodePty();
    if (!nodePty) {
      throw new ExecuteTaskError("node-pty is not available", { command: "<group>", errorCode: null });
    }

    const shellEnv = await getShellEnv(shellEnvOptions(this.options.config, this.options.logger));
    const children: GroupChild[] = [];
    const group: ProcessGroup = {
      terminal: this,
      spawn: (spec) => this.spawnInGroup(spec, children, shellEnv, nodePty),
    };

    this.userInterrupted = false;
    this.inGroup = true;
    try {
      const result = await callback(group);
      // Override clean exits when the user hit Ctrl+C (simctl swallowing SIGINT, Swift
      // apps returning 0 from a SIGINT handler, etc.).
      if (this.userInterrupted) {
        throw new ExecuteTaskError("Command was cancelled by user", {
          command: "<group>",
          errorCode: 130,
        });
      }
      return result;
    } catch (err) {
      // Same override on the throw path: if the user interrupted, that's the dominant
      // intent — devicectl exits non-zero after handling SIGINT, log sidecars may also
      // surface errors during teardown. We swallow those in favor of the cancellation.
      if (this.userInterrupted) {
        throw new ExecuteTaskError("Command was cancelled by user", {
          command: "<group>",
          errorCode: 130,
        });
      }
      throw err;
    } finally {
      this.inGroup = false;
      await this.cleanupGroup(children);
    }
  }

  private spawnInGroup(
    spec: ProcessSpec,
    children: GroupChild[],
    shellEnv: NodeJS.ProcessEnv,
    nodePty: NonNullable<ReturnType<typeof loadNodePty>>,
  ): ProcessHandle {
    if (spec.main && !spec.pty) {
      throw new ExecuteTaskError("ProcessSpec.main requires pty: true", {
        command: spec.command,
        errorCode: null,
      });
    }
    if (spec.main && this.mainPty) {
      throw new ExecuteTaskError("Group already has a main process", {
        command: spec.command,
        errorCode: null,
      });
    }
    const env = this.mergeEnv(shellEnv, spec.env);
    const cwd = spec.cwd ?? this.options.workspaceRoot.getPath();
    const args = spec.args ?? [];
    const child = spec.pty
      ? this.spawnPtyChild(spec, args, env, cwd, nodePty)
      : this.spawnPipeChild(spec, args, env, cwd);
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

  private spawnPtyChild(
    spec: ProcessSpec,
    args: string[],
    env: NodeJS.ProcessEnv,
    cwd: string,
    nodePty: NonNullable<ReturnType<typeof loadNodePty>>,
  ): GroupChild {
    const pty = nodePty.spawn(spec.command, args, {
      name: "xterm-256color",
      cwd,
      env: env as { [key: string]: string },
      cols: this.dims.cols,
      rows: this.dims.rows,
    });
    this.groupPtys.add(pty);
    if (spec.main) {
      this.mainPty = pty;
    }

    const dataListeners: ProcessOutputSink[] = [];
    pty.onData((chunk) => {
      for (const l of dataListeners) l(chunk);
    });

    let alive = true;
    const exit = new Promise<ProcessExit>((resolve) => {
      pty.onExit(({ exitCode, signal }) => {
        alive = false;
        this.groupPtys.delete(pty);
        if (this.mainPty === pty) {
          this.mainPty = undefined;
        }
        resolve({ code: exitCode ?? -1, signal: ptySignalName(signal) });
      });
    });

    return {
      pid: pty.pid,
      exit,
      get alive() {
        return alive;
      },
      signal: (sig) => {
        if (!alive) return;
        try {
          pty.kill(sig);
        } catch {}
      },
      onData: (l) => {
        dataListeners.push(l);
      },
      // PTY merges streams; stderr can't be observed separately.
      onError: () => {},
    };
  }

  private spawnPipeChild(spec: ProcessSpec, args: string[], env: NodeJS.ProcessEnv, cwd: string): GroupChild {
    // detached: true puts the child in its own process group so we can take down
    // the whole tree via process.kill(-pid, signal) — matches v2's pattern.
    const proc = childSpawn(spec.command, args, {
      cwd,
      env: env as { [key: string]: string },
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    });
    this.groupPipes.add(proc);

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
        this.groupPipes.delete(proc);
        resolve(result);
      };
      proc.on("close", (code, signal) => finish({ code: code ?? -1, signal }));
      proc.on("error", () => finish({ code: -1, signal: null }));
    });

    return {
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
          // ESRCH = process already gone; anything else is unexpected.
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
  }

  private async cleanupGroup(children: GroupChild[]): Promise<void> {
    const alive = children.filter((c) => c.alive);
    if (alive.length === 0) return;

    for (const c of alive) {
      try {
        c.signal("SIGTERM");
      } catch {}
    }

    await Promise.race([
      Promise.all(alive.map((c) => c.exit.catch(() => undefined))),
      new Promise<void>((resolve) => setTimeout(resolve, TERMINATE_TIMEOUT_MS)),
    ]);

    for (const c of children) {
      if (c.alive) {
        try {
          c.signal("SIGKILL");
        } catch {}
      }
    }
  }

  private killGroupChildren(): void {
    for (const p of this.groupPtys) {
      try {
        p.kill("SIGTERM");
      } catch {}
    }
    for (const c of this.groupPipes) {
      if (!c.pid) continue;
      try {
        process.kill(-c.pid, "SIGTERM");
      } catch (err) {
        if ((err as NodeJS.ErrnoException).code !== "ESRCH") {
          this.options.logger.debug("group pipe SIGTERM failed", {
            error: err instanceof Error ? err.message : String(err),
          });
        }
      }
    }
    setTimeout(() => {
      for (const p of this.groupPtys) {
        try {
          p.kill("SIGKILL");
        } catch {}
      }
      for (const c of this.groupPipes) {
        if (!c.pid) continue;
        try {
          process.kill(-c.pid, "SIGKILL");
        } catch {}
      }
    }, TERMINATE_TIMEOUT_MS);
  }

  private mergeEnv(
    shellEnv: NodeJS.ProcessEnv,
    overrides: { [key: string]: string | null } | undefined,
  ): NodeJS.ProcessEnv {
    const merged: NodeJS.ProcessEnv = { ...shellEnv };
    if (overrides) {
      const prepared = prepareEnvVars(overrides);
      for (const [key, value] of Object.entries(prepared)) {
        if (value === undefined) {
          delete merged[key];
        } else {
          merged[key] = value;
        }
      }
    }
    merged.TERM = merged.TERM || "xterm-256color";
    return merged;
  }

  private async buildExecuteEnv(options: CommandOptions): Promise<NodeJS.ProcessEnv> {
    const shellEnv = await getShellEnv(shellEnvOptions(this.options.config, this.options.logger));
    return this.mergeEnv(shellEnv, options.env);
  }

  // Terminate the pty's child with SIGTERM, then SIGKILL after a grace period.
  // Only called from close() when VS Code tears the terminal down (user
  // clicked trash, vscode.tasks.terminate, window shutdown, etc.) — the
  // happy-path build completion does not go through here.
  //
  // pty.kill(sig) signals a SINGLE PID (pty.pid), not the process group. With
  // closeStdin set (and always for the pipes path), pty.pid is the wrapper
  // bash; SIGTERM hits bash, bash exits, the pty hangs up, and the kernel
  // delivers SIGHUP to the foreground process group on the slave — that's
  // what actually kills xcodebuild/xcbeautify in those cases. The 2s grace
  // period gives them time to flush logs and shut down cleanly; SIGKILL is
  // the uncatchable fallback if anything ignored the SIGTERM.
  //
  // try/catch swallows ESRCH (process already exited). Fire-and-forget: the
  // actual exit is observed via the ProcessExit promise on the pty handle.
  private killCurrentPty(): void {
    if (!this.currentPty) return;
    const pty = this.currentPty;
    try {
      pty.kill("SIGTERM");
    } catch {}
    setTimeout(() => {
      try {
        pty.kill("SIGKILL");
      } catch {}
    }, TERMINATE_TIMEOUT_MS);
  }

  private async start(): Promise<void> {
    try {
      await this.options.callback(this);
    } catch (error) {
      this.handleTerminalError(error);
      return;
    }
    this.closeSuccessfully();
  }

  private closeSuccessfully(): void {
    this.closeTerminal(0, "✅ Task completed", { color: "green" });
  }

  private closeTerminal(code: number, message: string, options?: TerminalWriteOptions): void {
    this.writeLine(message, options);
    this.writeLine();
    if (this.closeFired) return;
    this.closeFired = true;
    this.closeEmitter.fire(code);
  }

  private handleTerminalError(error: unknown): void {
    let errorCode = -1;
    let errorMessage = `🚷 ${error?.toString()}`;
    const options: TerminalWriteOptions = { color: "red" };

    if (error instanceof ExecuteTaskError) {
      errorCode = error.errorCode ?? errorCode;
      errorMessage = `🚫 ${error.message}`;
      if (errorCode === 130) {
        options.color = "yellow";
        this.closeTerminal(0, "🫡 Command was cancelled by user", options);
        return;
      }
    }
    this.closeTerminal(errorCode, errorMessage, options);
  }
}

// `set -o pipefail; A | B | C` — every command token is shell-quoted (so user-supplied paths
// can't break out), and the operators (`|`, `< /dev/null`) are plain shell syntax we splice in,
// not quoted arguments. When closeStdin is set, A's stdin is redirected from /dev/null so it
// sees EOF instead of waiting on the inherited TTY; B, C, … get their stdin from the pipe.
function buildPipelineScript(
  command: string,
  args: string[],
  pipes: NonNullable<CommandOptions["pipes"]>,
  closeStdin: boolean,
): string {
  const main = quote([command, ...args]) + (closeStdin ? " < /dev/null" : "");
  const rest = pipes.map((p) => quote([p.command, ...(p.args ?? [])]));
  return `set -o pipefail; ${[main, ...rest].join(" | ")}`;
}

function quoteForDisplay(command: string, args: string[]): string {
  return quote([command, ...args]);
}

function stripAnsi(value: string): string {
  return value.replace(ANSI_STRIP_REGEX, "");
}

export async function runTaskV3<TMetadata>(
  deps: VsCodeTaskRunnerDeps,
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
  if (loadNodePty() === null) {
    return runTaskV2(deps, options);
  }

  if (options.terminateLocked) {
    const tasks = vscode.tasks.taskExecutions.filter((task) => task.task.definition.lockId === options.lock);
    for (const task of tasks) {
      task.terminate();
    }
  }

  const currentScope = deps.execution.getScope();
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
      return new TaskTerminalV3({
        callback: (terminal) => {
          return deps.execution.setScope(currentScope, () => {
            return options.callback(terminal);
          });
        },
        workspaceRoot: deps.workspaceRoot,
        config: deps.config,
        logger: deps.logger,
      });
    }),
    options.problemMatchers,
  );
  setTaskPresentationOptions(task);

  const taskExecution = await vscode.tasks.executeTask(task);

  return new Promise((resolve, reject) => {
    const disposable = vscode.tasks.onDidEndTaskProcess((e) => {
      if (e.execution === taskExecution) {
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
