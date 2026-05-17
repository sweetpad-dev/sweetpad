import { type ChildProcess, spawn } from "node:child_process";

import { quote } from "shell-quote";

import type { ConfigProvider } from "../../core/config/types";
import { prepareEnvVars } from "../../core/helpers";
import type { Logger } from "../../core/logger/types";
import { LineBuffer } from "../../core/tasks/line-buffer";
import { getShellEnv } from "../../core/tasks/shell-env";
import {
  type CommandOptions,
  ExecuteTaskError,
  type ProcessGroup,
  type TaskRunner,
  type TaskTerminal,
  type TerminalWriteOptions,
  cleanCommandArgs,
} from "../../core/tasks/types";
import type { WorkspaceRoot } from "../../core/workspace-root";

export type NodeTaskRunnerDeps = {
  workspaceRoot: WorkspaceRoot;
  config: ConfigProvider;
  logger: Logger;
};

/**
 * Headless `TaskRunner` for the server. Spawns child processes via plain
 * `child_process.spawn` — no `vscode.tasks`, no node-pty, no terminal panel.
 * Output lines are captured (for `onOutputLine`) and forwarded to the
 * server's logger; `write()` falls through to stderr so JSON stdout stays
 * clean for the CLI client.
 *
 * Lock semantics match the VS Code runner: at most one execution per lock; a
 * new call with `terminateLocked: true` SIGTERMs the in-flight one before
 * proceeding. `stopMatching` is the engine's stop-by-scheme path; it cancels
 * matching executions but does not wait for them to die.
 *
 * `runGroup` is unimplemented in v1 — only used by run/launch flows, which
 * the agent CLI doesn't expose yet.
 */
export class NodeTaskRunner implements TaskRunner {
  private readonly inFlight = new Map<string, NodeTaskExecution>();

  constructor(private readonly deps: NodeTaskRunnerDeps) {}

  async run<TMetadata>(options: {
    name: string;
    source?: string;
    error?: string;
    problemMatchers?: string[];
    lock: string;
    metadata?: TMetadata;
    terminateLocked: boolean;
    callback: (terminal: TaskTerminal) => Promise<void>;
  }): Promise<void> {
    if (options.terminateLocked) {
      const existing = this.inFlight.get(options.lock);
      if (existing) {
        existing.cancel("SIGTERM");
        // Give the previous execution a chance to settle before we take over —
        // matching the VS Code runner's "stop first, then start" flow.
        await existing.completion.catch(() => {});
      }
    }

    const execution = new NodeTaskExecution({
      ...this.deps,
      metadata: options.metadata as Record<string, unknown> | undefined,
    });
    this.inFlight.set(options.lock, execution);
    try {
      await execution.start(options.callback);
    } finally {
      if (this.inFlight.get(options.lock) === execution) {
        this.inFlight.delete(options.lock);
      }
    }
  }

  stopMatching(predicate: { lock: string; metadata?: Record<string, unknown> }): void {
    for (const [lock, execution] of this.inFlight) {
      if (lock !== predicate.lock) continue;
      if (predicate.metadata && !matchesMetadata(execution.metadata, predicate.metadata)) continue;
      execution.cancel("SIGTERM");
    }
  }
}

function matchesMetadata(
  candidate: Record<string, unknown> | undefined,
  predicate: Record<string, unknown>,
): boolean {
  if (!candidate) return false;
  for (const [key, value] of Object.entries(predicate)) {
    if (candidate[key] !== value) return false;
  }
  return true;
}

/** One execution = one engine task (build/clean/test/...). Holds the active child process, if any. */
class NodeTaskExecution implements TaskTerminal {
  readonly metadata: Record<string, unknown> | undefined;
  private readonly workspaceRoot: WorkspaceRoot;
  private readonly config: ConfigProvider;
  private readonly logger: Logger;
  private currentProcess: ChildProcess | undefined;
  private cancelled = false;
  /** Resolves when the engine callback returns (or throws). */
  readonly completion: Promise<void>;
  private completionResolve!: () => void;
  private completionReject!: (err: unknown) => void;

  constructor(options: NodeTaskRunnerDeps & { metadata: Record<string, unknown> | undefined }) {
    this.workspaceRoot = options.workspaceRoot;
    this.config = options.config;
    this.logger = options.logger;
    this.metadata = options.metadata;
    this.completion = new Promise<void>((resolve, reject) => {
      this.completionResolve = resolve;
      this.completionReject = reject;
    });
    // Only the cancel-then-restart path awaits this promise. For the normal
    // path nobody attaches a `.catch`, so a rejection would surface as an
    // unhandled rejection and (in Node 21+) crash the server. Attach a no-op
    // — the main `start()` path still rethrows the same error to its caller,
    // so this swallow is purely for the orphan-completion case.
    this.completion.catch(() => {});
  }

  async start(callback: (terminal: TaskTerminal) => Promise<void>): Promise<void> {
    try {
      await callback(this);
      this.completionResolve();
    } catch (error) {
      this.completionReject(error);
      throw error;
    }
  }

  cancel(signal: NodeJS.Signals): void {
    this.cancelled = true;
    const proc = this.currentProcess;
    if (proc?.pid) {
      try {
        // Kill the entire process group (we spawn detached so xcodebuild + xcbeautify
        // share a pgid). Negative PID is the conventional way to signal the group.
        process.kill(-proc.pid, signal);
      } catch (error) {
        const code = (error as NodeJS.ErrnoException).code;
        if (code !== "ESRCH") {
          this.logger.debug("Process group signal failed", {
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }
    }
  }

  async execute(options: CommandOptions): Promise<void> {
    const args = cleanCommandArgs(options.args);
    const commandPrint = quote([options.command, ...args]);

    const shellEnv = await getShellEnv({
      logger: this.logger,
      shell: this.config.get("shellEnv.shell"),
      timeoutMs: this.config.get("shellEnv.timeout"),
      cwd: this.workspaceRoot.getPath(),
    });
    const env = { ...shellEnv, ...prepareEnvVars(options.env ?? {}) };

    const stdoutBuffer = new LineBuffer({
      enabled: !!options.onOutputLine,
      callback: (line) => {
        void options.onOutputLine?.({ value: line, type: "stdout" });
      },
    });
    const stderrBuffer = new LineBuffer({
      enabled: !!options.onOutputLine,
      callback: (line) => {
        void options.onOutputLine?.({ value: line, type: "stderr" });
      },
    });

    return await new Promise<void>((resolve, reject) => {
      const cwd = options.cwd ?? this.workspaceRoot.getPath();

      // Two spawn shapes:
      //   1. Pipeline (pipes present): /bin/bash -c 'set -o pipefail; CMD < /dev/null? | xcbeautify'
      //      Shell parses the pipeline; pipefail surfaces the upstream exit code.
      //   2. Single command: spawn(command, args, ...).
      //      argv array form keeps argument boundaries intact (paths with spaces, etc.).
      const usePipeline = !!options.pipes && options.pipes.length > 0;
      const stdinSetting = options.closeStdin || usePipeline ? "ignore" : "inherit";

      let child: ChildProcess;
      if (usePipeline) {
        const main = options.closeStdin
          ? `${quote([options.command, ...args])} < /dev/null`
          : quote([options.command, ...args]);
        const pipeline = [main, ...(options.pipes ?? []).map((p) => quote([p.command, ...(p.args ?? [])]))].join(" | ");
        child = spawn("/bin/bash", ["-c", `set -o pipefail; ${pipeline}`], {
          cwd,
          env: env as { [key: string]: string },
          stdio: [stdinSetting, "pipe", "pipe"],
          detached: true,
        });
      } else {
        child = spawn(options.command, args, {
          cwd,
          env: env as { [key: string]: string },
          stdio: [stdinSetting, "pipe", "pipe"],
          detached: true,
        });
      }

      this.currentProcess = child;
      this.logger.debug("NodeTaskRunner spawn", {
        command: options.command,
        args,
        cwd,
        pipeline: usePipeline,
      });

      child.stdout?.on("data", (chunk: Buffer) => stdoutBuffer.append(chunk.toString("utf8")));
      child.stderr?.on("data", (chunk: Buffer) => stderrBuffer.append(chunk.toString("utf8")));

      child.once("error", (error) => {
        stdoutBuffer.flush();
        stderrBuffer.flush();
        this.currentProcess = undefined;
        reject(
          new ExecuteTaskError(`Failed to spawn '${options.command}': ${error.message}`, {
            command: commandPrint,
            errorCode: null,
          }),
        );
      });

      child.once("close", (code, signal) => {
        stdoutBuffer.flush();
        stderrBuffer.flush();
        this.currentProcess = undefined;

        if (this.cancelled) {
          reject(
            new ExecuteTaskError("Command cancelled", {
              command: commandPrint,
              errorCode: code,
            }),
          );
          return;
        }

        if (code !== 0) {
          reject(
            new ExecuteTaskError(
              signal
                ? `Command terminated by ${signal}`
                : `Command exited with code ${code ?? "<unknown>"}`,
              { command: commandPrint, errorCode: code },
            ),
          );
          return;
        }

        resolve();
      });
    });
  }

  write(data: string, _options?: TerminalWriteOptions): void {
    // The CLI listens on stdout for the JSON envelope. Engine-level "write"
    // calls (progress chatter, "App launched on device", etc.) go to stderr
    // so they don't corrupt the data contract.
    process.stderr.write(data);
  }

  async runGroup<T>(_callback: (group: ProcessGroup) => Promise<T>): Promise<T> {
    // Only used by run/launch flows (runOnMac, runOniOSSimulator, runOniOSDevice).
    // The CLI's first slice is `sweetpad build`, which never enters those paths.
    // Implement when adding `sweetpad run`.
    throw new ExecuteTaskError("runGroup is not implemented in the headless task runner yet", {
      command: "<runGroup>",
      errorCode: null,
    });
  }
}
